package io.github.singularity098.yoink

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Settings, in SharedPreferences. The GitHub token is encrypted with a key
 * that lives in the Android Keystore (the phone's keyring) and never
 * leaves it.
 */
class Settings(context: Context) {
    private val prefs = context.getSharedPreferences("settings", Context.MODE_PRIVATE)
    private val saved = context.getSharedPreferences("saved_apks", Context.MODE_PRIVATE)

    var theme: Int
        get() = prefs.getInt("theme", 2)
        set(v) = prefs.edit().putInt("theme", v).apply()

    /** Take Actions builds from failed runs too. */
    var allowFailed: Boolean
        get() = prefs.getBoolean("allow_failed", false)
        set(v) = prefs.edit().putBoolean("allow_failed", v).apply()

    /** The folder (a document tree URI) installed APKs are copied to; null: none. */
    var apkFolder: String?
        get() = prefs.getString("apk_folder", null)
        set(v) = prefs.edit().apply { if (v == null) remove("apk_folder") else putString("apk_folder", v) }.apply()

    /** The name of the APK last saved to the folder for app [id], to replace it with the next. */
    fun savedApk(id: String): String? = saved.getString(id, null)

    fun setSavedApk(id: String, name: String?) =
        saved.edit().apply { if (name == null) remove(id) else putString(id, name) }.apply()

    var token: String?
        get() = prefs.getString("token", null)?.let { runCatching { decrypt(it) }.getOrNull() }
        set(v) {
            if (v.isNullOrBlank()) prefs.edit().remove("token").apply()
            else prefs.edit().putString("token", encrypt(v.trim())).apply()
        }

    private fun key(): SecretKey {
        val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (ks.getEntry(ALIAS, null) as? KeyStore.SecretKeyEntry)?.let { return it.secretKey }
        val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        gen.init(
            KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .build()
        )
        return gen.generateKey()
    }

    private fun encrypt(text: String): String {
        val c = Cipher.getInstance("AES/GCM/NoPadding")
        c.init(Cipher.ENCRYPT_MODE, key())
        val out = c.iv + c.doFinal(text.toByteArray())
        return Base64.encodeToString(out, Base64.NO_WRAP)
    }

    private fun decrypt(stored: String): String {
        val bytes = Base64.decode(stored, Base64.NO_WRAP)
        val c = Cipher.getInstance("AES/GCM/NoPadding")
        c.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, bytes, 0, 12))
        return c.doFinal(bytes, 12, bytes.size - 12).decodeToString()
    }

    private companion object {
        const val ALIAS = "yoink-token"
    }
}
