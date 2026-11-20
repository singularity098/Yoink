package io.github.singularity098.yoink

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.os.Build
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.asSharedFlow
import java.io.File
import java.util.concurrent.ConcurrentHashMap

/**
 * Installs APKs with Android's PackageInstaller. Android shows its own
 * confirmation (except for updates of apps Yoink installed, on Android 12
 * and newer); the outcome comes back to [InstallReceiver].
 */
object Installer {
    sealed class Outcome {
        data object Success : Outcome()
        data class Failure(val message: String) : Outcome()
    }

    private const val ACTION = "io.github.singularity098.yoink.INSTALL_STATUS"
    private val waiting = ConcurrentHashMap<Int, CompletableDeferred<Outcome>>()

    /** Android's confirmation screens, for the activity to show. */
    private val _confirmations = MutableSharedFlow<Intent>(extraBufferCapacity = 4)
    val confirmations = _confirmations.asSharedFlow()

    suspend fun install(context: Context, apk: File, packageName: String): Outcome {
        val installer = context.packageManager.packageInstaller
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL).apply {
            setAppPackageName(packageName)
            setSize(apk.length())
            if (Build.VERSION.SDK_INT >= 31) {
                setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_NOT_REQUIRED)
            }
        }
        val id = installer.createSession(params)
        val done = CompletableDeferred<Outcome>()
        waiting[id] = done
        try {
            installer.openSession(id).use { session ->
                session.openWrite("base.apk", 0, apk.length()).use { out ->
                    apk.inputStream().use { it.copyTo(out, 256 * 1024) }
                    session.fsync(out)
                }
                val intent = Intent(context, InstallReceiver::class.java).setAction(ACTION)
                val flags = PendingIntent.FLAG_UPDATE_CURRENT or
                    (if (Build.VERSION.SDK_INT >= 31) PendingIntent.FLAG_MUTABLE else 0)
                val pending = PendingIntent.getBroadcast(context, id, intent, flags)
                session.commit(pending.intentSender)
            }
        } catch (e: Exception) {
            waiting.remove(id)
            runCatching { installer.abandonSession(id) }
            return Outcome.Failure("could not hand the APK to Android: ${e.message}")
        }
        return done.await()
    }

    internal fun onStatus(context: Context, intent: Intent) {
        val id = intent.getIntExtra(PackageInstaller.EXTRA_SESSION_ID, -1)
        val status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)
        when (status) {
            PackageInstaller.STATUS_PENDING_USER_ACTION -> {
                @Suppress("DEPRECATION")
                val confirm = (if (Build.VERSION.SDK_INT >= 33)
                    intent.getParcelableExtra(Intent.EXTRA_INTENT, Intent::class.java)
                else intent.getParcelableExtra(Intent.EXTRA_INTENT)) ?: return
                confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                // The activity shows it; if it isn't listening, start it from here.
                if (!_confirmations.tryEmit(confirm) || _confirmations.subscriptionCount.value == 0) {
                    runCatching { context.startActivity(confirm) }
                }
            }
            PackageInstaller.STATUS_SUCCESS -> waiting.remove(id)?.complete(Outcome.Success)
            else -> {
                val message = intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE)
                waiting.remove(id)?.complete(Outcome.Failure(describe(status, message)))
            }
        }
    }

    private fun describe(status: Int, message: String?): String {
        val what = when (status) {
            PackageInstaller.STATUS_FAILURE_ABORTED -> "the install was cancelled"
            PackageInstaller.STATUS_FAILURE_BLOCKED -> "Android blocked the install"
            PackageInstaller.STATUS_FAILURE_CONFLICT ->
                "it conflicts with the installed app: signed with another key? Uninstall that one first"
            PackageInstaller.STATUS_FAILURE_INCOMPATIBLE -> "the APK is not compatible with this phone"
            PackageInstaller.STATUS_FAILURE_INVALID -> "the APK is invalid"
            PackageInstaller.STATUS_FAILURE_STORAGE -> "not enough storage"
            else -> "the install failed"
        }
        return if (message.isNullOrBlank() || status == PackageInstaller.STATUS_FAILURE_ABORTED) what else "$what ($message)"
    }
}

class InstallReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) = Installer.onStatus(context, intent)
}
