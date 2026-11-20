package io.github.singularity098.yoink

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream

/** The same cases as the desktop app's tests (src/pattern.rs, src/forge), plus picking APKs. */
class CoreTest {
    @Test
    fun globs() {
        assertTrue(Pattern.matches("Chippy-*-x86_64.AppImage", "Chippy-0.2.0-x86_64.AppImage"))
        assertFalse(Pattern.matches("Chippy-*-x86_64.AppImage", "Chippy-0.2.0-aarch64.AppImage"))
        assertTrue(Pattern.matches("*", ""))
        assertTrue(Pattern.matches("a?c", "abc"))
        assertFalse(Pattern.matches("a?c", "ac"))
        assertTrue(Pattern.matches("*.tar.gz", "x.tar.gz"))
        assertTrue(Pattern.matches("a*b*c", "aXXbYYc"))
        assertFalse(Pattern.matches("a*b*c", "aXXbYY"))
        assertTrue(Pattern.matches("exact", "exact"))
    }

    @Test
    fun derives() {
        assertEquals("Chippy-*-x86_64.AppImage", Pattern.derive("Chippy-0.1.0-x86_64.AppImage", "v0.1.0"))
        assertEquals("tool-v*-linux.tar.gz", Pattern.derive("tool-v1.2.3-linux.tar.gz"))
        assertEquals("chippy-linux-x86_64.tar.gz", Pattern.derive("chippy-linux-x86_64.tar.gz", "v0.1.0"))
        assertEquals("yoink-linux-*", Pattern.derive("yoink-linux-3f2a9c1"))
        assertEquals("build-*-amd64.zip", Pattern.derive("build-20261119-amd64.zip"))
        assertEquals("app_*_amd64.AppImage", Pattern.derive("app_2.0.0-rc1_amd64.AppImage"))
        assertEquals("nightly-x86_64.AppImage", Pattern.derive("nightly-x86_64.AppImage", "nightly"))
        val p = Pattern.derive("Surge-XT-1.3.1-linux-x86_64.tar.gz", "release_xt_1.3.1")
        assertTrue(p, Pattern.matches(p, "Surge-XT-1.3.4-linux-x86_64.tar.gz"))
        assertEquals("app-*-arm64-v8a-release.apk", Pattern.derive("app-1.4.2-arm64-v8a-release.apk", "v1.4.2"))
    }

    @Test
    fun checksums() {
        val h = "a".repeat(64)
        val g = "b".repeat(64)
        val text = "$h  app.tar.gz\n$g *other.zip\n"
        assertEquals(h, GitHub.findChecksum(text, "app.tar.gz"))
        assertEquals(g, GitHub.findChecksum(text, "other.zip"))
        assertNull(GitHub.findChecksum(text, "none"))
        assertEquals(h, GitHub.findChecksum("$h\n", "x"))
        assertEquals(h, GitHub.findChecksum("$h  dist/app.tar.gz", "app.tar.gz"))
    }

    @Test
    fun sources() {
        assertEquals("a" to "b", GitHub.parseSource("a/b"))
        assertEquals("singularity098" to "Yoink", GitHub.parseSource("https://github.com/singularity098/Yoink.git"))
        assertEquals("o" to "r", GitHub.parseSource("github.com/o/r/releases/tag/v1"))
        assertEquals("o" to "r", GitHub.parseSource("git@github.com:o/r.git"))
        assertNull(GitHub.parseSource("nope"))
        assertNull(GitHub.parseSource("a b/c"))
    }

    @Test
    fun channelsAndRemotesRoundTrip() {
        for (c in listOf(Channel.Releases(), Channel.Releases(true), Channel.Actions("android.yml", "dev"))) {
            assertEquals(c, Channel.fromJson(c.toJson()))
        }
        val r = Remote("k", "v1", "a.apk", 3, "u", "application/octet-stream", false, null, "", "w")
        assertEquals(r, Remote.fromJson(r.toJson()))
        assertEquals(r.copy(checksumUrl = "c"), Remote.fromJson(r.copy(checksumUrl = "c").toJson()))
    }

    private val phone = listOf("arm64-v8a", "armeabi-v7a", "armeabi")

    @Test
    fun picksTheApkForThisPhone() {
        val assets = listOf(
            "app-1.0-x86_64.apk", "app-1.0-armeabi-v7a.apk", "app-1.0-arm64-v8a.apk",
            "app-1.0-x86.apk", "app-1.0.AppImage", "SHA256SUMS.txt",
        )
        assertEquals("app-1.0-arm64-v8a.apk", Apk.best(assets, phone))
        assertEquals("app-1.0-x86_64.apk", Apk.best(assets, listOf("x86_64", "x86")))
        // A universal APK beats one for no ABI in particular, and anything beats unsigned.
        assertEquals("app-universal.apk", Apk.best(listOf("app.apk", "app-universal.apk"), phone))
        assertEquals("app-release.apk", Apk.best(listOf("app-release-unsigned.apk", "app-release.apk"), phone))
        // The phone's own ABI beats a universal build.
        assertEquals("app-arm64.apk", Apk.best(listOf("app-universal.apk", "app-arm64.apk"), phone))
        assertNull(Apk.best(listOf("app.AppImage", "app.tar.gz"), phone))
        assertNull(Apk.best(listOf("app-x86_64.apk"), phone))
        assertNull(Apk.score("app-x86.apk", listOf("x86_64")))
        // Actions artifacts: names without .apk, the Android one first.
        assertEquals("app-android", Apk.best(listOf("app-linux", "app-android", "app-windows.zip"), phone, requireApk = false))
    }

    @Test
    fun extractsTheApkFromAZip() {
        val dir = kotlin.io.path.createTempDirectory("yoink").toFile()
        try {
            val zip = File(dir, "artifact.zip")
            ZipOutputStream(zip.outputStream()).use { z ->
                for (n in listOf("out/app-x86_64-release.apk", "out/app-arm64-v8a-release.apk", "README.txt")) {
                    z.putNextEntry(ZipEntry(n)); z.write(n.toByteArray()); z.closeEntry()
                }
            }
            val (apk, inner) = Apk.extract(zip, "", phone)
            assertEquals("app-arm64-v8a-release.apk", inner)
            assertEquals("out/app-arm64-v8a-release.apk", apk.readText())
            // The remembered pattern wins.
            val (apk2, _) = Apk.extract(zip, "app-x86_64-*.apk", phone)
            assertEquals("out/app-x86_64-release.apk", apk2.readText())

            // An APK is a zip too: it is used as it is.
            val plain = File(dir, "plain.bin")
            ZipOutputStream(plain.outputStream()).use { z ->
                z.putNextEntry(ZipEntry("AndroidManifest.xml")); z.write(1); z.closeEntry()
            }
            assertEquals(plain to null, Apk.extract(plain, "", phone))
        } finally {
            dir.deleteRecursively()
        }
    }
}
