package io.github.singularity098.yoink

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.DocumentsContract
import java.io.File
import java.io.IOException

/**
 * A folder the user picked (with Android's folder picker) to keep a copy of
 * each installed APK in: for sharing, syncing or backing them up. Like the
 * desktop app's APK folder, each app's new APK replaces its previous one.
 */
object ApkFolder {
    private const val MIME = "application/vnd.android.package-archive"

    /** Keeps the access Android granted to [tree] across restarts. */
    fun remember(context: Context, tree: Uri) {
        context.contentResolver.takePersistableUriPermission(
            tree, Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION,
        )
    }

    fun forget(context: Context, tree: Uri) {
        runCatching {
            context.contentResolver.releasePersistableUriPermission(
                tree, Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION,
            )
        }
    }

    /** Whether Yoink may still write to [tree] (the user can revoke it, or remove the storage). */
    fun usable(context: Context, tree: Uri): Boolean =
        context.contentResolver.persistedUriPermissions.any { it.uri == tree && it.isWritePermission }

    /** The folder's name, as the picker showed it. */
    fun describe(context: Context, tree: Uri): String {
        val doc = runCatching {
            DocumentsContract.buildDocumentUriUsingTree(tree, DocumentsContract.getTreeDocumentId(tree))
        }.getOrNull()
        return doc?.let { displayName(context, it) } ?: tree.lastPathSegment?.substringAfterLast(':')?.ifEmpty { null } ?: "the chosen folder"
    }

    /** The files directly in [tree]: name to document. */
    private fun children(context: Context, tree: Uri): Map<String, Uri> {
        val parent = DocumentsContract.getTreeDocumentId(tree)
        val list = DocumentsContract.buildChildDocumentsUriUsingTree(tree, parent)
        val cols = arrayOf(DocumentsContract.Document.COLUMN_DOCUMENT_ID, DocumentsContract.Document.COLUMN_DISPLAY_NAME)
        val out = mutableMapOf<String, Uri>()
        context.contentResolver.query(list, cols, null, null, null)?.use { c ->
            while (c.moveToNext()) {
                val name = c.getString(1) ?: continue
                out[name] = DocumentsContract.buildDocumentUriUsingTree(tree, c.getString(0))
            }
        }
        return out
    }

    /**
     * Copies [apk] into [tree] as [name] (overwriting a file of that name),
     * and deletes [previous], the app's last saved APK, if it is another
     * file. Returns the name it was saved as.
     */
    fun save(context: Context, tree: Uri, apk: File, name: String, previous: String?): String {
        val resolver = context.contentResolver
        val existing = children(context, tree)
        val target = existing[name]
            ?: DocumentsContract.createDocument(
                resolver,
                DocumentsContract.buildDocumentUriUsingTree(tree, DocumentsContract.getTreeDocumentId(tree)),
                MIME, name,
            )
            ?: throw IOException("could not create $name")
        // "wt": truncate, so a shorter APK doesn't keep the old one's tail.
        (resolver.openOutputStream(target, "wt") ?: throw IOException("could not write $name")).use { out ->
            apk.inputStream().use { it.copyTo(out, 256 * 1024) }
        }
        // Providers may rename a new file to avoid a clash ("x (1).apk").
        val saved = existing[name]?.let { name } ?: displayName(context, target) ?: name
        if (previous != null && previous != saved) {
            existing[previous]?.let { runCatching { DocumentsContract.deleteDocument(resolver, it) } }
        }
        return saved
    }

    private fun displayName(context: Context, doc: Uri): String? = runCatching {
        context.contentResolver.query(doc, arrayOf(DocumentsContract.Document.COLUMN_DISPLAY_NAME), null, null, null)?.use {
            if (it.moveToFirst()) it.getString(0) else null
        }
    }.getOrNull()

    /** The name to save an APK as: its own, made to end in .apk (a zip that is itself the APK). */
    fun nameFor(fileName: String): String {
        val base = fileName.substringAfterLast('/')
        return when {
            base.lowercase().endsWith(".apk") -> base
            base.lowercase().endsWith(".zip") -> base.dropLast(4) + ".apk"
            else -> "$base.apk"
        }
    }
}
