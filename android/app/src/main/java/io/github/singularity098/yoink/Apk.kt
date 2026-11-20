package io.github.singularity098.yoink

import java.io.File
import java.io.IOException
import java.util.zip.ZipFile

/** Picking the right APK for this phone, and getting it out of a zip. */
object Apk {
    /** Names an ABI goes by in file names. */
    private val ALIASES = mapOf(
        "arm64-v8a" to listOf("arm64-v8a", "arm64", "aarch64", "armv8"),
        "armeabi-v7a" to listOf("armeabi-v7a", "armeabi", "armv7", "arm32", "armhf"),
        "x86_64" to listOf("x86_64", "x86-64", "x64", "amd64"),
        "x86" to listOf("x86", "i686", "i386"),
    )

    private fun mentions(lower: String, alias: String): Boolean {
        var from = 0
        while (true) {
            val i = lower.indexOf(alias, from)
            if (i < 0) return false
            val before = if (i == 0) ' ' else lower[i - 1]
            val after = if (i + alias.length >= lower.length) ' ' else lower[i + alias.length]
            // "x86" must not match inside "x86_64" (or "x86-64").
            val rest = lower.substring(i + alias.length)
            val sepBefore = !before.isLetterOrDigit()
            val sepAfter = !after.isLetterOrDigit() && !(alias == "x86" && (rest.startsWith("_64") || rest.startsWith("-64")))
            if (sepBefore && sepAfter) return true
            from = i + 1
        }
    }

    /**
     * How likely [name] is to be the build for a phone with [abis] (most
     * preferred first). Null: not an APK, or for another ABI.
     * [requireApk] false also scores zips and Actions artifacts by name.
     */
    fun score(name: String, abis: List<String>, requireApk: Boolean = true): Int? {
        val lower = name.lowercase()
        val isApk = lower.endsWith(".apk")
        if (requireApk && !isApk) return null
        if (lower.endsWith(".apks") || lower.endsWith(".xapk") || lower.endsWith(".aab")) return null
        if (lower.contains("sha256") || lower.endsWith(".sig") || lower.endsWith(".asc")) return null
        var score = if (isApk) 100 else when {
            lower.endsWith(".zip") || !lower.contains('.') -> 0
            else -> return null
        }
        if (!isApk && (lower.contains("apk") || lower.contains("android"))) score += 50
        val named = ALIASES.entries.filter { (_, aliases) -> aliases.any { mentions(lower, it) } }.map { it.key }
        if (named.isEmpty()) {
            score += if (lower.contains("universal")) 40 else 30
        } else {
            val best = named.minOf { abis.indexOf(it).let { i -> if (i < 0) Int.MAX_VALUE else i } }
            if (best == Int.MAX_VALUE) return null
            score += 45 - 10 * best
        }
        if (lower.contains("unsigned")) score -= 60
        if (lower.contains("debug")) score -= 5
        return score
    }

    /** The likeliest of [names] for this phone, if any is likely at all. */
    fun best(names: List<String>, abis: List<String>, requireApk: Boolean = true): String? =
        names.mapNotNull { n -> score(n, abis, requireApk)?.let { n to it } }.maxByOrNull { it.second }?.first

    fun isZip(file: File): Boolean = file.inputStream().use { s ->
        val head = ByteArray(4)
        s.read(head) == 4 && head[0] == 'P'.code.toByte() && head[1] == 'K'.code.toByte()
    }

    /**
     * The APK to install from [file]: the file itself if it is one, or the
     * APK inside it (by [innerPattern], or the likeliest), extracted next to
     * it. Returns the APK and its name inside the zip (null: not from a zip).
     */
    fun extract(file: File, innerPattern: String, abis: List<String>): Pair<File, String?> {
        if (!isZip(file)) throw IOException("${file.name} is neither an APK nor a zip")
        ZipFile(file).use { zip ->
            val names = zip.entries().asSequence().filter { !it.isDirectory }.map { it.name }.toList()
            if ("AndroidManifest.xml" in names) return file to null
            val apks = names.filter { it.lowercase().endsWith(".apk") }
            if (apks.isEmpty()) {
                val bundles = names.any { it.lowercase().let { n -> n.endsWith(".apks") || n.endsWith(".xapk") || n.endsWith(".aab") } }
                throw IOException(
                    if (bundles) "${file.name} holds a split APK bundle, which Yoink can't install yet"
                    else "${file.name} has no APK in it"
                )
            }
            val byPattern = innerPattern.takeIf { it.isNotEmpty() }?.let { p ->
                apks.firstOrNull { Pattern.matches(p, it.substringAfterLast('/')) }
            }
            val chosen = byPattern ?: best(apks.map { it }, abis)
                ?: throw IOException("${file.name} has no APK for this phone (${apks.joinToString { it.substringAfterLast('/') }})")
            val out = File(file.parentFile, chosen.substringAfterLast('/'))
            zip.getInputStream(zip.getEntry(chosen)).use { input -> out.outputStream().use { input.copyTo(it) } }
            return out to chosen.substringAfterLast('/')
        }
    }
}
