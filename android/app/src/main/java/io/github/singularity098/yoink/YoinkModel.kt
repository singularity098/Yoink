package io.github.singularity098.yoink

import android.app.Application
import android.content.pm.PackageInfo
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.withContext
import java.io.File
import java.io.IOException
import java.security.MessageDigest
import java.util.UUID

/** What an app is doing right now. */
data class Busy(val label: String, val done: Long = 0, val total: Long = -1, val cancellable: Boolean = false)

/** The installed package, as Android reports it. */
data class Present(val versionName: String, val versionCode: Long, val lastUpdate: Long)

/** One app in a plan: what will be downloaded and installed. */
data class PlanItem(val app: AppEntry, val remote: Remote, val reinstall: Boolean)

enum class Status { NotInstalled, UpdateAvailable, UpToDate, Unknown }

class YoinkModel(app: Application) : AndroidViewModel(app) {
    private val context get() = getApplication<Application>()
    private val pm: PackageManager get() = context.packageManager
    val settings = Settings(app)
    private val library = Library(File(app.filesDir, "library.json"))
    private val staging = File(app.cacheDir, "staging")
    private val abis: List<String> = Build.SUPPORTED_ABIS.toList()

    var apps by mutableStateOf(library.load())
        private set
    val busy = mutableStateMapOf<String, Busy>()
    val present = mutableStateMapOf<String, Present>()
    var checking by mutableStateOf(false)
        private set
    /** A plan waiting for the user to apply it. */
    var plan by mutableStateOf<List<PlanItem>?>(null)
        private set
    var planning by mutableStateOf(false)
        private set
    var message by mutableStateOf<String?>(null)
    var theme by mutableStateOf(settings.theme)
        private set
    var hasToken by mutableStateOf(settings.token != null)
        private set
    var allowFailed by mutableStateOf(settings.allowFailed)
        private set
    var editor by mutableStateOf<Editor?>(null)
        private set
    /** The folder installed APKs are also saved to, by name; null: none. */
    var apkFolderName by mutableStateOf<String?>(null)
        private set
    /** The folder is set but Yoink lost access to it (revoked, or its storage removed). */
    var apkFolderLost by mutableStateOf(false)
        private set
    /** Whether the user lets Yoink install apps (Settings → Install unknown apps). */
    var canInstall by mutableStateOf(false)
        private set
    /** Apps whose uninstall Android is asking about: removed once the package is gone. */
    private val uninstalling = mutableSetOf<String>()
    private val cancelled = mutableSetOf<String>()
    private val installLock = Mutex()

    init {
        refreshPresent()
        refreshApkFolder()
        staging.deleteRecursively()
    }

    private fun github(app: AppEntry) = GitHub(app.owner, app.repo, settings.token)

    private fun update(id: String, change: (AppEntry) -> AppEntry) {
        apps = apps.map { if (it.id == id) change(it) else it }
        library.save(apps)
    }

    fun app(id: String) = apps.firstOrNull { it.id == id }

    fun status(app: AppEntry): Status {
        val p = present[app.packageName]
        return when {
            app.installed == null || app.packageName.isEmpty() || p == null -> Status.NotInstalled
            app.latest == null -> Status.Unknown
            app.latest.key != app.installed.key -> Status.UpdateAvailable
            else -> Status.UpToDate
        }
    }

    val updatable get() = apps.filter { status(it) == Status.UpdateAvailable }

    // ----------------------------------------------------------- the phone

    private fun packageInfo(name: String): PackageInfo? = runCatching {
        if (Build.VERSION.SDK_INT >= 33) pm.getPackageInfo(name, PackageManager.PackageInfoFlags.of(0))
        else @Suppress("DEPRECATION") pm.getPackageInfo(name, 0)
    }.getOrNull()

    @Suppress("DEPRECATION")
    private fun PackageInfo.code(): Long = if (Build.VERSION.SDK_INT >= 28) longVersionCode else versionCode.toLong()

    /** Rereads which managed packages are installed; settles installs and uninstalls done meanwhile. */
    fun refreshPresent() {
        canInstall = pm.canRequestPackageInstalls()
        present.clear()
        for (a in apps) {
            if (a.packageName.isEmpty()) continue
            packageInfo(a.packageName)?.let { present[a.packageName] = Present(it.versionName.orEmpty(), it.code(), it.lastUpdateTime) }
        }
        // An install Android finished while Yoink wasn't listening (Yoink updating itself).
        for (a in apps) {
            val pending = a.pending ?: continue
            val p = present[a.packageName]
            if (p != null && p.versionCode == pending.versionCode && p.lastUpdate >= pending.at - 60_000) {
                update(a.id) { it.copy(installed = pending, pending = null) }
            } else if (System.currentTimeMillis() - pending.at > 30 * 60_000) {
                update(a.id) { it.copy(pending = null) }
            }
        }
        for (id in uninstalling.toList()) {
            val a = app(id)
            uninstalling.remove(id)
            if (a != null && present[a.packageName] == null) {
                removeEntry(id)
                message = "${a.name} uninstalled"
            }
        }
    }

    // ------------------------------------------------------------ checking

    fun checkAll() {
        if (checking || apps.isEmpty()) return
        checking = true
        viewModelScope.launch {
            val limit = Semaphore(4)
            apps.map { a -> async { limit.withPermit { check(a.id) } } }.awaitAll()
            refreshPresent()
            checking = false
            val n = updatable.size
            message = when {
                apps.any { it.error.isNotEmpty() } -> null
                n == 0 -> "Everything is up to date"
                n == 1 -> "1 update"
                else -> "$n updates"
            }
        }
    }

    fun checkOne(id: String) {
        viewModelScope.launch {
            check(id)
            refreshPresent()
        }
    }

    private suspend fun check(id: String): Remote? {
        val a = app(id) ?: return null
        busy[id] = Busy("Checking…")
        return try {
            val remote = withContext(Dispatchers.IO) { github(a).resolve(a.channel, a.pattern, settings.allowFailed) }
            update(id) { it.copy(latest = remote, error = "") }
            remote
        } catch (e: Exception) {
            update(id) { it.copy(error = e.message ?: e.toString()) }
            null
        } finally {
            busy.remove(id)
        }
    }

    // ------------------------------------------------------------ updating

    /** Works out what updating [ids] would download and install, for [plan]. */
    fun prepare(ids: List<String>, reinstall: Boolean = false) {
        if (planning) return
        planning = true
        viewModelScope.launch {
            val items = mutableListOf<PlanItem>()
            for (id in ids) {
                val a = app(id) ?: continue
                // A forced reinstall, or a first install, looks again for the newest build.
                val remote = if (reinstall || a.latest == null || status(a) == Status.NotInstalled) check(id) else a.latest
                if (remote != null) items += PlanItem(app(id) ?: a, remote, reinstall)
            }
            planning = false
            // Yoink itself last: installing it restarts it.
            plan = items.sortedBy { it.app.packageName == context.packageName }.takeIf { it.isNotEmpty() }
            if (plan == null) message = "Nothing to install: ${ids.mapNotNull { app(it)?.error?.takeIf(String::isNotEmpty) }.firstOrNull() ?: "no build found"}"
        }
    }

    fun dismissPlan() {
        plan = null
    }

    fun apply() {
        val items = plan ?: return
        plan = null
        viewModelScope.launch {
            installLock.withLock {
                var done = 0
                for (item in items) {
                    if (install(item)) done++
                }
                refreshPresent()
                if (items.size > 1) message = "$done of ${items.size} installed"
            }
        }
    }

    fun cancel(id: String) {
        cancelled += id
    }

    /** Downloads, checks and installs one build. */
    private suspend fun install(item: PlanItem): Boolean {
        val id = item.app.id
        val a = app(id) ?: return false
        val remote = item.remote
        val dir = File(staging, id)
        cancelled.remove(id)
        try {
            busy[id] = Busy("Downloading", 0, remote.size, cancellable = true)
            val gh = github(a)
            val file = File(dir, remote.fileName)
            withContext(Dispatchers.IO) {
                dir.deleteRecursively()
                gh.download(remote, file, { id in cancelled }) { done, total ->
                    busy[id] = Busy("Downloading", done, if (total > 0) total else remote.size, cancellable = true)
                }
            }
            busy[id] = Busy("Checking the download…")
            val checksumNote = withContext(Dispatchers.IO) { verify(gh, remote, file) }
            val (apk, inner) = withContext(Dispatchers.IO) {
                if (remote.fileName.lowercase().endsWith(".apk")) file to null else Apk.extract(file, a.innerPattern, abis)
            }
            val info = archiveInfo(apk) ?: throw IOException("${apk.name} is not an APK Android can read")
            val pkg = info.packageName
            if (a.packageName.isNotEmpty() && a.packageName != pkg) {
                throw IOException("this build is $pkg, not ${a.packageName}: edit the app to pick another file")
            }
            packageInfo(pkg)?.let { now ->
                if (now.code() > info.code()) {
                    throw IOException(
                        "the installed version (${now.versionName}, ${now.code()}) is newer than this build " +
                            "(${info.versionName}, ${info.code()}): Android won't downgrade; uninstall it first"
                    )
                }
            }
            val label = info.applicationInfo?.let {
                it.sourceDir = apk.path
                it.publicSourceDir = apk.path
                it.loadLabel(pm).toString()
            }
            val record = Installed(
                key = remote.key, version = remote.version, fileName = remote.fileName,
                versionName = info.versionName.orEmpty(), versionCode = info.code(), at = System.currentTimeMillis(),
            )
            update(id) {
                it.copy(
                    packageName = pkg,
                    name = label?.takeIf(String::isNotBlank) ?: it.name,
                    innerPattern = inner?.let { n -> it.innerPattern.ifEmpty { Pattern.derive(n) } } ?: it.innerPattern,
                    pending = record,
                )
            }
            val savedNote = saveCopy(id, apk)
            busy[id] = Busy("Installing…")
            when (val outcome = Installer.install(context, apk, pkg)) {
                is Installer.Outcome.Success -> {
                    update(id) { it.copy(installed = record, pending = null, error = "") }
                    message = "${label ?: a.name} ${info.versionName.orEmpty()} installed$checksumNote$savedNote"
                    return true
                }
                is Installer.Outcome.Failure -> {
                    update(id) { it.copy(pending = null) }
                    throw IOException(outcome.message)
                }
            }
        } catch (e: CancelledError) {
            message = "${a.name}: cancelled"
            return false
        } catch (e: Exception) {
            val text = e.message ?: e.toString()
            update(id) { it.copy(error = text) }
            message = "${a.name}: $text"
            return false
        } finally {
            busy.remove(id)
            withContext(Dispatchers.IO) { dir.deleteRecursively() }
            refreshPresent()
        }
    }

    /**
     * Copies the APK to the user's APK folder, if one is set, replacing the
     * app's previous one. Returns a note for the message; never fails the install.
     */
    private suspend fun saveCopy(id: String, apk: File): String {
        val tree = settings.apkFolder?.let(Uri::parse) ?: return ""
        busy[id] = Busy("Saving the APK…")
        return withContext(Dispatchers.IO) {
            try {
                if (!ApkFolder.usable(context, tree)) throw IOException("Yoink no longer has access to the folder")
                val name = ApkFolder.save(context, tree, apk, ApkFolder.nameFor(apk.name), settings.savedApk(id))
                settings.setSavedApk(id, name)
                ""
            } catch (e: Exception) {
                "; the APK was not saved to the folder: ${e.message ?: e}"
            }
        }
    }

    /** Checks the SHA-256 when the release publishes one; returns a note for the message. */
    private fun verify(gh: GitHub, remote: Remote, file: File): String {
        val url = remote.checksumUrl ?: return ""
        val text = runCatching { gh.fetchSmall(url).decodeToString() }.getOrNull() ?: return " (checksum file unreadable)"
        val want = GitHub.findChecksum(text, remote.fileName) ?: return ""
        val digest = MessageDigest.getInstance("SHA-256")
        file.inputStream().use { s ->
            val buf = ByteArray(256 * 1024)
            while (true) {
                val n = s.read(buf)
                if (n < 0) break
                digest.update(buf, 0, n)
            }
        }
        val got = digest.digest().joinToString("") { "%02x".format(it) }
        if (got != want) throw IOException("the SHA-256 of ${remote.fileName} does not match the published one")
        return ", SHA-256 verified"
    }

    private fun archiveInfo(apk: File): PackageInfo? =
        if (Build.VERSION.SDK_INT >= 33) pm.getPackageArchiveInfo(apk.path, PackageManager.PackageInfoFlags.of(0))
        else @Suppress("DEPRECATION") pm.getPackageArchiveInfo(apk.path, 0)

    // ------------------------------------------------------------ removing

    fun stopManaging(id: String) {
        val a = app(id) ?: return
        removeEntry(id)
        message = "Yoink no longer manages ${a.name}"
    }

    /** The package to ask Android to uninstall; the entry goes once it's gone. */
    fun beginUninstall(id: String): String? {
        val a = app(id) ?: return null
        if (a.packageName.isEmpty() || present[a.packageName] == null) {
            removeEntry(id)
            return null
        }
        uninstalling += id
        return a.packageName
    }

    private fun removeEntry(id: String) {
        apps = apps.filter { it.id != id }
        library.save(apps)
        library.iconFile(id).delete()
        settings.setSavedApk(id, null)
    }

    fun iconFile(id: String) = library.iconFile(id)

    // ------------------------------------------------------------- options

    fun chooseTheme(i: Int) {
        theme = i
        settings.theme = i
    }

    fun setToken(t: String?) {
        settings.token = t
        hasToken = settings.token != null
        message = if (hasToken) "Token saved" else "Token removed"
    }

    fun changeAllowFailed(v: Boolean) {
        allowFailed = v
        settings.allowFailed = v
    }

    fun refreshApkFolder() {
        val tree = settings.apkFolder?.let(Uri::parse)
        if (tree == null) {
            apkFolderName = null
            apkFolderLost = false
            return
        }
        apkFolderLost = !ApkFolder.usable(context, tree)
        apkFolderName = if (apkFolderLost) "the chosen folder" else ApkFolder.describe(context, tree)
    }

    /** The folder the user picked for saving APKs (from Android's folder picker); null: stop saving. */
    fun setApkFolder(tree: Uri?) {
        settings.apkFolder?.let(Uri::parse)?.takeIf { it != tree }?.let { ApkFolder.forget(context, it) }
        if (tree == null) {
            settings.apkFolder = null
            refreshApkFolder()
            message = "APKs are no longer saved to a folder"
            return
        }
        try {
            ApkFolder.remember(context, tree)
        } catch (e: Exception) {
            message = "Android did not give Yoink lasting access to that folder"
            return
        }
        // Names saved in another folder mean nothing in this one.
        if (settings.apkFolder != tree.toString()) apps.forEach { settings.setSavedApk(it.id, null) }
        settings.apkFolder = tree.toString()
        refreshApkFolder()
        message = "APKs will be saved to $apkFolderName"
    }

    // -------------------------------------------------------------- editor

    fun openEditor(id: String?) {
        val e = Editor(id)
        app(id ?: "")?.let { a ->
            e.repoText = a.source
            e.channelKind = when (val c = a.channel) {
                is Channel.Releases -> if (c.prerelease) 1 else 0
                is Channel.Actions -> 2
            }
            (a.channel as? Channel.Actions)?.let { e.workflow = it.workflow; e.branch = it.branch }
            e.pattern = a.pattern
            e.keepPattern = true
            lookUp(e)
        }
        editor = e
    }

    fun closeEditor() {
        editor = null
    }

    fun lookUp(e: Editor) {
        val (owner, repo) = GitHub.parseSource(e.repoText) ?: run {
            e.error = "Type owner/name, or paste the repository's address"
            return
        }
        e.error = ""
        e.info = null
        e.offer = null
        e.busy = true
        viewModelScope.launch {
            try {
                val gh = GitHub(owner, repo, settings.token)
                e.info = withContext(Dispatchers.IO) { gh.info() }
                e.owner = owner
                e.repo = repo
                loadOffer(e)
            } catch (ex: Exception) {
                e.error = ex.message ?: ex.toString()
                e.busy = false
            }
        }
    }

    fun loadOffer(e: Editor) {
        if (e.info == null) return
        e.busy = true
        e.offer = null
        e.error = ""
        viewModelScope.launch {
            try {
                val gh = GitHub(e.owner, e.repo, settings.token)
                val offer = withContext(Dispatchers.IO) { gh.offer(e.channel) }
                e.offer = offer
                if (!e.keepPattern || offer.files.none { Pattern.matches(e.pattern, it.name) }) {
                    val names = offer.files.map { it.name }
                    val best = Apk.best(names, abis) ?: Apk.best(names, abis, requireApk = false)
                    if (best != null) e.pick(best) else { e.picked = null; e.pattern = "" }
                } else {
                    e.picked = offer.files.firstOrNull { Pattern.matches(e.pattern, it.name) }?.name
                }
                e.keepPattern = false
            } catch (ex: Exception) {
                e.error = ex.message ?: ex.toString()
            } finally {
                e.busy = false
            }
        }
    }

    fun saveEditor(e: Editor) {
        val info = e.info ?: return
        if (e.pattern.isBlank()) return
        val existing = e.editingId?.let { app(it) }
        val entry = if (existing != null) {
            val changed = existing.channel != e.channel || existing.pattern != e.pattern ||
                existing.owner != e.owner || existing.repo != e.repo
            existing.copy(
                owner = e.owner, repo = e.repo, channel = e.channel, pattern = e.pattern.trim(),
                latest = if (changed) null else existing.latest,
                innerPattern = if (changed) "" else existing.innerPattern,
                error = "",
            )
        } else {
            AppEntry(
                id = UUID.randomUUID().toString(), owner = e.owner, repo = e.repo,
                name = info.name.ifEmpty { e.repo }, channel = e.channel, pattern = e.pattern.trim(),
            )
        }
        apps = if (existing != null) apps.map { if (it.id == entry.id) entry else it } else apps + entry
        library.save(apps)
        editor = null
        viewModelScope.launch {
            if (!library.iconFile(entry.id).exists()) {
                withContext(Dispatchers.IO) {
                    GitHub(entry.owner, entry.repo, settings.token).avatar(info)?.let { bytes ->
                        library.iconFile(entry.id).apply { parentFile?.mkdirs(); writeBytes(bytes) }
                    }
                }
                apps = apps.toList() // redraw with the icon
            }
            check(entry.id)
        }
    }
}

/** Adding or editing an app. */
class Editor(val editingId: String?) {
    var repoText by mutableStateOf("")
    var owner by mutableStateOf("")
    var repo by mutableStateOf("")
    var info by mutableStateOf<RepoInfo?>(null)
    var busy by mutableStateOf(false)
    var error by mutableStateOf("")
    /** 0 Releases, 1 Releases + pre-releases, 2 Actions. */
    var channelKind by mutableStateOf(0)
    var workflow by mutableStateOf("")
    var branch by mutableStateOf("")
    var offer by mutableStateOf<Offer?>(null)
    var picked by mutableStateOf<String?>(null)
    var pattern by mutableStateOf("")
    /** Editing: keep the saved pattern if the offer still matches it. */
    var keepPattern = false

    val channel: Channel
        get() = when (channelKind) {
            2 -> Channel.Actions(workflow.trim(), branch.trim())
            else -> Channel.Releases(channelKind == 1)
        }

    fun pick(name: String) {
        picked = name
        pattern = Pattern.derive(name, offer?.tag)
    }

    /** The offered files the pattern matches. */
    val matching: List<Candidate>
        get() = offer?.files?.filter { Pattern.matches(pattern.trim(), it.name) }.orEmpty()
}
