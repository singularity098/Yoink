package io.github.singularity098.yoink

import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/** What Yoink last installed for an app. */
data class Installed(
    /** The [Remote.key] installed: a different one is an update. */
    val key: String,
    /** The release tag or run, as shown. */
    val version: String,
    val fileName: String,
    val versionName: String,
    val versionCode: Long,
    val at: Long,
) {
    fun toJson(): JSONObject = JSONObject().put("key", key).put("version", version).put("file_name", fileName)
        .put("version_name", versionName).put("version_code", versionCode).put("at", at)

    companion object {
        fun fromJson(o: JSONObject?): Installed? = o?.let {
            Installed(
                it.optString("key"), it.optString("version"), it.optString("file_name"),
                it.optString("version_name"), it.optLong("version_code"), it.optLong("at"),
            )
        }
    }
}

/**
 * An app Yoink manages: one APK from one channel of one repository. The
 * desktop app's "several deployments" are several entries here (a phone
 * build from Releases and a dev build from Actions, say).
 */
data class AppEntry(
    val id: String,
    val owner: String,
    val repo: String,
    /** As shown: the repository's name, then the installed app's label. */
    val name: String,
    val channel: Channel,
    /** The release asset or Actions artifact, as a glob. */
    val pattern: String,
    /** The APK inside a zip (Actions artifacts, zipped releases), as a glob; empty: the likeliest. */
    val innerPattern: String = "",
    /** The Android package, once known (after the first install). */
    val packageName: String = "",
    val installed: Installed? = null,
    /** The newest build found by the last check. */
    val latest: Remote? = null,
    /** What went wrong in the last check, if anything. */
    val error: String = "",
    /** An install handed to Android and not yet confirmed (Yoink may be restarted meanwhile). */
    val pending: Installed? = null,
) {
    val source get() = "$owner/$repo"
    val web get() = "https://github.com/$owner/$repo"

    fun toJson(): JSONObject = JSONObject()
        .put("id", id).put("owner", owner).put("repo", repo).put("name", name)
        .put("channel", channel.toJson()).put("pattern", pattern).put("inner_pattern", innerPattern)
        .put("package", packageName)
        .put("installed", installed?.toJson() ?: JSONObject.NULL)
        .put("latest", latest?.toJson() ?: JSONObject.NULL)
        .put("error", error)
        .put("pending", pending?.toJson() ?: JSONObject.NULL)

    companion object {
        fun fromJson(o: JSONObject) = AppEntry(
            id = o.getString("id"),
            owner = o.getString("owner"),
            repo = o.getString("repo"),
            name = o.optString("name").ifEmpty { o.getString("repo") },
            channel = Channel.fromJson(o.optJSONObject("channel")),
            pattern = o.optString("pattern"),
            innerPattern = o.optString("inner_pattern"),
            packageName = o.optString("package"),
            installed = Installed.fromJson(o.optJSONObject("installed")),
            latest = Remote.fromJson(o.optJSONObject("latest")),
            error = o.optString("error"),
            pending = Installed.fromJson(o.optJSONObject("pending")),
        )
    }
}

/** The apps, in `files/library.json`. */
class Library(private val file: File) {
    fun load(): List<AppEntry> = runCatching {
        if (!file.exists()) return emptyList()
        val arr = JSONObject(file.readText()).optJSONArray("apps") ?: JSONArray()
        (0 until arr.length()).mapNotNull { i -> runCatching { AppEntry.fromJson(arr.getJSONObject(i)) }.getOrNull() }
    }.getOrDefault(emptyList())

    fun save(apps: List<AppEntry>) {
        val arr = JSONArray()
        apps.forEach { arr.put(it.toJson()) }
        val text = JSONObject().put("version", 1).put("apps", arr).toString(2)
        val tmp = File(file.path + ".tmp")
        tmp.writeText(text)
        if (!tmp.renameTo(file)) {
            file.writeText(text)
            tmp.delete()
        }
    }

    /** Where an app's icon is kept until its APK is installed (the owner's avatar). */
    fun iconFile(id: String) = File(file.parentFile, "icons/$id.png")
}
