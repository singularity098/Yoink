package io.github.singularity098.yoink

import org.json.JSONObject
import java.io.File
import java.io.IOException
import java.io.InputStream
import java.net.HttpURLConnection
import java.net.URL

/** An HTTP error worth reading: GitHub's message, and when its rate limit resets. */
class HttpError(val status: Int, message: String) : IOException(message)

class CancelledError : IOException("cancelled")

/**
 * HTTPS, as the desktop app does it (src/http.rs): redirects followed by
 * hand, so a token sent to GitHub's API never travels on to the storage
 * host a download redirects to.
 */
object Http {
    val USER_AGENT = "yoink-android/${BuildConfig.VERSION_NAME}"

    /** Hosts the token may be sent to. */
    private val TOKEN_HOSTS = setOf("api.github.com")

    fun tokenFor(url: String, token: String?): String? {
        if (token.isNullOrEmpty()) return null
        val host = runCatching { URL(url).host.lowercase() }.getOrNull() ?: return null
        return if (host in TOKEN_HOSTS) token else null
    }

    /** A GET that follows up to 8 redirects; returns an open connection with status < 300. */
    fun open(url: String, accept: String, token: String?): HttpURLConnection {
        var current = url
        repeat(8) {
            val c = URL(current).openConnection() as HttpURLConnection
            c.instanceFollowRedirects = false
            c.connectTimeout = 20_000
            c.readTimeout = 60_000
            c.setRequestProperty("User-Agent", USER_AGENT)
            c.setRequestProperty("Accept", accept)
            c.setRequestProperty("X-GitHub-Api-Version", "2022-11-28")
            tokenFor(current, token)?.let { c.setRequestProperty("Authorization", "Bearer $it") }
            val code = try {
                c.responseCode
            } catch (e: IOException) {
                c.disconnect()
                throw IOException("could not reach ${hostOf(current)}: ${e.message}", e)
            }
            when {
                code in 300..399 -> {
                    val next = c.getHeaderField("Location")
                        ?: throw IOException("$current: redirect without a location")
                    c.disconnect()
                    current = URL(URL(current), next).toString()
                }
                code >= 400 -> throw statusError(code, c, current)
                else -> return c
            }
        }
        throw IOException("$url: too many redirects")
    }

    private fun hostOf(url: String) = runCatching { URL(url).host }.getOrDefault(url)

    private fun statusError(code: Int, c: HttpURLConnection, url: String): HttpError {
        val remaining = c.getHeaderField("x-ratelimit-remaining")
        val reset = c.getHeaderField("x-ratelimit-reset")?.toLongOrNull()
        val body = runCatching { c.errorStream?.use { it.readBytes().decodeToString() } }.getOrNull().orEmpty()
        c.disconnect()
        val apiMessage = runCatching { JSONObject(body).optString("message") }.getOrNull()?.takeIf { it.isNotEmpty() }
        val message = if ((code == 403 || code == 429) && remaining == "0") {
            val wait = reset?.let {
                val mins = ((it - System.currentTimeMillis() / 1000).coerceAtLeast(0) + 59) / 60
                " (resets in $mins min)"
            }.orEmpty()
            "GitHub's rate limit was reached$wait; a token in Options raises it"
        } else when (code) {
            401 -> "GitHub did not accept the token (401): check it in Options"
            404 -> "not found (404): ${apiMessage ?: "a private repository needs a token"}"
            else -> "${hostOf(url)} answered $code${apiMessage?.let { ": $it" }.orEmpty()}"
        }
        return HttpError(code, message)
    }

    fun getText(url: String, token: String?, accept: String = "application/vnd.github+json"): String {
        val c = open(url, accept, token)
        try {
            return c.inputStream.use { it.readBytes().decodeToString() }
        } finally {
            c.disconnect()
        }
    }

    fun getJson(url: String, token: String?): JSONObject = try {
        JSONObject(getText(url, token))
    } catch (e: org.json.JSONException) {
        throw IOException("unexpected answer from $url", e)
    }

    fun getBytes(url: String, accept: String, token: String?, limit: Int): ByteArray {
        val c = open(url, accept, token)
        try {
            return c.inputStream.use { readLimited(it, limit) }
        } finally {
            c.disconnect()
        }
    }

    private fun readLimited(input: InputStream, limit: Int): ByteArray {
        val out = java.io.ByteArrayOutputStream()
        val buf = ByteArray(64 * 1024)
        while (out.size() < limit) {
            val n = input.read(buf, 0, minOf(buf.size, limit - out.size()))
            if (n < 0) break
            out.write(buf, 0, n)
        }
        return out.toByteArray()
    }

    /**
     * Downloads to [dest] (through `dest.part`), calling [progress] with
     * (done, total or -1) as it goes. [cancelled] returning true stops it.
     */
    fun download(
        url: String,
        accept: String,
        token: String?,
        dest: File,
        cancelled: () -> Boolean,
        progress: (Long, Long) -> Unit,
    ) {
        val c = open(url, accept, token)
        try {
            val total = c.contentLengthLong
            dest.parentFile?.mkdirs()
            val part = File(dest.path + ".part")
            var done = 0L
            progress(0, total)
            c.inputStream.use { input ->
                part.outputStream().use { out ->
                    val buf = ByteArray(256 * 1024)
                    while (true) {
                        if (cancelled()) {
                            out.close(); part.delete()
                            throw CancelledError()
                        }
                        val n = input.read(buf)
                        if (n < 0) break
                        out.write(buf, 0, n)
                        done += n
                        progress(done, total)
                    }
                    out.fd.sync()
                }
            }
            if (total >= 0 && total != done) {
                part.delete()
                throw IOException("the download was cut short ($done of $total bytes)")
            }
            dest.delete()
            if (!part.renameTo(dest)) throw IOException("could not move the download into place")
        } finally {
            c.disconnect()
        }
    }
}
