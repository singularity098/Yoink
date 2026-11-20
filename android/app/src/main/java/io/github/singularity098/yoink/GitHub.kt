package io.github.singularity098.yoink

import org.json.JSONArray
import org.json.JSONObject
import java.io.IOException
import java.net.URLEncoder

/** Where an app's builds come from: Releases, or Actions runs. */
sealed class Channel {
    /** The newest release (or pre-release, if asked). */
    data class Releases(val prerelease: Boolean = false) : Channel()

    /** The newest run of [workflow] (its file name; empty for any) on [branch] (empty: the default). */
    data class Actions(val workflow: String = "", val branch: String = "") : Channel()

    val label: String
        get() = when (this) {
            is Releases -> if (prerelease) "Releases + pre-releases" else "Releases"
            is Actions -> buildString {
                append("Actions")
                if (workflow.isNotEmpty()) append(" · $workflow")
                if (branch.isNotEmpty()) append(" · $branch")
            }
        }

    fun toJson(): JSONObject = when (this) {
        is Releases -> JSONObject().put("type", "releases").put("prerelease", prerelease)
        is Actions -> JSONObject().put("type", "actions").put("workflow", workflow).put("branch", branch)
    }

    companion object {
        fun fromJson(o: JSONObject?): Channel = when (o?.optString("type")) {
            "actions" -> Actions(o.optString("workflow"), o.optString("branch"))
            else -> Releases(o?.optBoolean("prerelease") ?: false)
        }
    }
}

/** The build an app would install now. */
data class Remote(
    /** A different key from the installed one is an update. */
    val key: String,
    /** A release tag, or a run number and commit. */
    val version: String,
    val fileName: String,
    val size: Long,
    val url: String,
    val accept: String,
    /** Only downloadable with a token (Actions artifacts). */
    val needsToken: Boolean,
    /** Where a SHA-256 for the file was published, if anywhere. */
    val checksumUrl: String?,
    /** Something worth knowing ("a newer build is in progress"). */
    val note: String,
    /** The release or run's page. */
    val web: String,
) {
    fun toJson(): JSONObject = JSONObject()
        .put("key", key).put("version", version).put("file_name", fileName).put("size", size)
        .put("url", url).put("accept", accept).put("needs_token", needsToken)
        .put("checksum_url", checksumUrl ?: JSONObject.NULL).put("note", note).put("web", web)

    companion object {
        fun fromJson(o: JSONObject?): Remote? = o?.let {
            Remote(
                key = it.optString("key"), version = it.optString("version"),
                fileName = it.optString("file_name"), size = it.optLong("size"),
                url = it.optString("url"), accept = it.optString("accept"),
                needsToken = it.optBoolean("needs_token"),
                checksumUrl = if (it.isNull("checksum_url")) null else it.optString("checksum_url"),
                note = it.optString("note"), web = it.optString("web"),
            )
        }
    }
}

/** A file a channel offers. */
data class Candidate(val name: String, val size: Long)

/** What a channel offers now, for picking. */
data class Offer(val files: List<Candidate>, val version: String, val note: String, val tag: String?)

data class RepoInfo(
    val name: String,
    val description: String,
    val defaultBranch: String,
    val avatarUrl: String,
    /** The workflows' file names. */
    val workflows: List<String>,
)

/** GitHub: releases, Actions runs and their artifacts (src/forge/github.rs). */
class GitHub(val owner: String, val repo: String, private val token: String?) {
    private var repoJson: JSONObject? = null

    private fun api(path: String) = "$API/repos/$owner/$repo$path"

    private fun repoJson(): JSONObject {
        repoJson?.let { return it }
        val r = try {
            Http.getJson(api(""), token)
        } catch (e: IOException) {
            throw IOException("$owner/$repo: ${e.message}", e)
        }
        repoJson = r
        return r
    }

    private fun releases(): List<JSONObject> =
        Http.getText(api("/releases?per_page=$LOOK_BACK"), token).let { JSONArray(it).objects() }

    private fun runs(workflow: String, branch: String): List<JSONObject> {
        val b = branch.ifEmpty { repoJson().getString("default_branch") }
        val base = if (workflow.isEmpty()) api("/actions/runs")
        else api("/actions/workflows/${enc(workflow)}/runs")
        return Http.getJson("$base?branch=${enc(b)}&per_page=$LOOK_BACK", token)
            .optJSONArray("workflow_runs").objects()
    }

    private fun artifacts(run: Long): List<JSONObject> =
        Http.getJson(api("/actions/runs/$run/artifacts?per_page=100"), token)
            .optJSONArray("artifacts").objects()

    private fun releaseWanted(r: JSONObject, prerelease: Boolean) =
        !r.optBoolean("draft") && (prerelease || !r.optBoolean("prerelease"))

    /** Through the API (works for private repositories) with a token, the public link without. */
    private fun assetUrl(a: JSONObject): String =
        if (token.isNullOrEmpty()) a.getString("browser_download_url") else a.getString("url")

    private fun checksumFor(r: JSONObject, asset: JSONObject): String? {
        val assets = r.optJSONArray("assets").objects()
        val lower = asset.getString("name").lowercase()
        val own = setOf("$lower.sha256", "$lower.sha256sum")
        fun shared(n: String) = n.contains("sha256") && (n.contains("sum") || n.endsWith(".txt")) ||
            n == "checksums.txt" || n.endsWith("checksums.txt")
        val found = assets.firstOrNull { it.getString("name").lowercase() in own }
            ?: assets.firstOrNull { it.getLong("id") != asset.getLong("id") && shared(it.getString("name").lowercase()) }
        return found?.let { assetUrl(it) }
    }

    fun info(): RepoInfo {
        val r = repoJson()
        // A repository without Actions answers 404: that is just none.
        val workflows = runCatching {
            Http.getJson(api("/actions/workflows?per_page=100"), token).optJSONArray("workflows").objects()
                .filter { it.optString("state") == "active" }
                .map { it.optString("path").substringAfterLast('/') }
                .filter { it.endsWith(".yml") || it.endsWith(".yaml") }
        }.getOrDefault(emptyList())
        return RepoInfo(
            name = r.optString("name"),
            description = r.optString("description").takeUnless { r.isNull("description") }.orEmpty(),
            defaultBranch = r.optString("default_branch"),
            avatarUrl = r.optJSONObject("owner")?.optString("avatar_url").orEmpty(),
            workflows = workflows,
        )
    }

    fun offer(channel: Channel): Offer = when (channel) {
        is Channel.Releases -> {
            val r = releases().firstOrNull {
                releaseWanted(it, channel.prerelease) && it.optJSONArray("assets").objects().isNotEmpty()
            } ?: throw IOException("no releases with files")
            Offer(
                files = r.optJSONArray("assets").objects().map { Candidate(it.getString("name"), it.optLong("size")) },
                version = r.getString("tag_name"),
                note = "",
                tag = r.getString("tag_name"),
            )
        }
        is Channel.Actions -> {
            var found: Offer? = null
            for (run in runs(channel.workflow, channel.branch).take(8)) {
                val arts = artifacts(run.getLong("id")).filter { !it.optBoolean("expired") }
                if (arts.isEmpty()) continue
                found = Offer(
                    files = arts.map { Candidate(it.getString("name"), it.optLong("size_in_bytes")) },
                    version = "run #${run.optLong("run_number")} (${run.optString("head_sha").take(7)})",
                    note = if (run.optString("status") != "completed")
                        "this run is still in progress: more files may come" else "",
                    tag = null,
                )
                break
            }
            found ?: throw IOException("no artifacts in the recent runs${where(channel.workflow, channel.branch)}")
        }
    }

    fun resolve(channel: Channel, pattern: String, allowFailed: Boolean): Remote = when (channel) {
        is Channel.Releases -> resolveRelease(channel.prerelease, pattern)
        is Channel.Actions -> resolveActions(channel.workflow, channel.branch, pattern, allowFailed)
    }

    private fun resolveRelease(prerelease: Boolean, pattern: String): Remote {
        val wanted = releases().filter { releaseWanted(it, prerelease) }
        if (wanted.isEmpty()) throw IOException("no ${if (prerelease) "" else "stable "}releases published")
        wanted.forEachIndexed { i, r ->
            val asset = r.optJSONArray("assets").objects().firstOrNull { Pattern.matches(pattern, it.getString("name")) }
                ?: return@forEachIndexed
            val tag = r.getString("tag_name")
            return Remote(
                key = "release:$tag:${asset.getLong("id")}:${asset.optString("updated_at")}",
                version = tag,
                fileName = asset.getString("name"),
                size = asset.optLong("size"),
                url = assetUrl(asset),
                accept = "application/octet-stream",
                needsToken = false,
                checksumUrl = checksumFor(r, asset),
                note = if (i > 0) "the newest release (${wanted[0].getString("tag_name")}) has no file matching $pattern" else "",
                web = r.optString("html_url"),
            )
        }
        throw IOException("no file matching $pattern in the last ${wanted.size} releases")
    }

    private fun resolveActions(workflow: String, branch: String, pattern: String, allowFailed: Boolean): Remote {
        val runs = runs(workflow, branch)
        if (runs.isEmpty()) throw IOException("no Actions runs found${where(workflow, branch)}")
        var inProgress: JSONObject? = null
        var skippedFailed: JSONObject? = null
        for (run in runs) {
            val running = run.optString("status") != "completed"
            val conclusion = if (run.isNull("conclusion")) "" else run.optString("conclusion")
            if (!running && conclusion != "success" && !(allowFailed && conclusion == "failure")) {
                if (conclusion == "failure" && skippedFailed == null) skippedFailed = run
                continue
            }
            val a = artifacts(run.getLong("id"))
                .firstOrNull { !it.optBoolean("expired") && Pattern.matches(pattern, it.getString("name")) }
            if (a == null) {
                // Still building (this file not uploaded yet): the one before is what there is.
                if (running && inProgress == null) inProgress = run
                continue
            }
            val number = run.optLong("run_number")
            val sha = run.optString("head_sha")
            val notes = buildList {
                inProgress?.let { add("a newer build (#${it.optLong("run_number")}) is in progress") }
                if (running) add("from run #$number, still in progress (this file is finished)")
                if (conclusion == "failure") add("run #$number failed")
            }
            return Remote(
                key = "actions:${run.getLong("id")}:${a.getLong("id")}:$sha",
                version = "#$number (${sha.take(7)})",
                fileName = "${a.getString("name")}.zip",
                size = a.optLong("size_in_bytes"),
                url = a.getString("archive_download_url"),
                accept = "application/vnd.github+json",
                needsToken = true,
                checksumUrl = null,
                note = notes.joinToString("; "),
                web = run.optString("html_url"),
            )
        }
        inProgress?.let {
            throw IOException("run #${it.optLong("run_number")} is in progress and $pattern is not uploaded yet; no earlier build has it")
        }
        skippedFailed?.let {
            throw IOException("no successful run has $pattern (run #${it.optLong("run_number")} failed; Options can allow failed runs)")
        }
        throw IOException("no artifact matching $pattern in the last ${runs.size} runs${where(workflow, branch)} (artifacts expire after 90 days)")
    }

    fun fetchSmall(url: String): ByteArray = Http.getBytes(url, "application/octet-stream", token, 4_000_000)

    fun avatar(info: RepoInfo): ByteArray? {
        if (info.avatarUrl.isEmpty()) return null
        val sep = if ('?' in info.avatarUrl) '&' else '?'
        return runCatching { Http.getBytes("${info.avatarUrl}${sep}s=192", "image/*", token, 2_000_000) }.getOrNull()
    }

    fun download(remote: Remote, dest: java.io.File, cancelled: () -> Boolean, progress: (Long, Long) -> Unit) {
        if (remote.needsToken && token.isNullOrEmpty()) {
            throw IOException("GitHub only lets Actions builds be downloaded with a token: add one in Options")
        }
        Http.download(remote.url, remote.accept, token, dest, cancelled, progress)
    }

    companion object {
        private const val API = "https://api.github.com"
        /** How far back to look for a release or run with the file wanted. */
        private const val LOOK_BACK = 15

        fun where(workflow: String, branch: String) = when {
            workflow.isEmpty() && branch.isEmpty() -> " on the default branch"
            branch.isEmpty() -> " of $workflow on the default branch"
            workflow.isEmpty() -> " on $branch"
            else -> " of $workflow on $branch"
        }

        fun enc(s: String): String = URLEncoder.encode(s, "UTF-8").replace("+", "%20")

        /**
         * `owner/repo`, `github.com/owner/repo` or a GitHub URL (with or
         * without `.git` and anything after the repository).
         */
        fun parseSource(text: String): Pair<String, String>? {
            var s = text.trim().trimEnd('/')
            for (p in listOf("https://", "http://", "git@")) s = s.removePrefix(p)
            for (p in listOf("www.github.com/", "github.com/", "github.com:")) s = s.removePrefix(p)
            val parts = s.split('/')
            if (parts.size < 2) return null
            val owner = parts[0].trim()
            val repo = parts[1].trim().removeSuffix(".git")
            fun ok(p: String) = p.isNotEmpty() && p.all { it.isLetterOrDigit() && it.code < 128 || it in "-_." }
            return if (ok(owner) && ok(repo)) owner to repo else null
        }

        /**
         * The SHA-256 for [fileName] in a checksum file: `hash  name` lines
         * (sha256sum's format, `*name` too), or a lone hash.
         */
        fun findChecksum(text: String, fileName: String): String? {
            fun isHash(s: String) = s.length == 64 && s.all { it.isDigit() || it.lowercaseChar() in 'a'..'f' }
            var lone: String? = null
            var lines = 0
            for (raw in text.lines()) {
                val line = raw.trim()
                if (line.isEmpty()) continue
                lines++
                val parts = line.split(Regex("\\s+"))
                val first = parts[0]
                if (!isHash(first)) continue
                if (parts.size == 1) {
                    lone = first.lowercase()
                } else {
                    val name = parts.drop(1).joinToString(" ").trimStart('*')
                    if (name.substringAfterLast('/') == fileName) return first.lowercase()
                }
            }
            return if (lines == 1) lone else null
        }
    }
}

private fun JSONArray?.objects(): List<JSONObject> =
    if (this == null) emptyList() else (0 until length()).mapNotNull { optJSONObject(it) }
