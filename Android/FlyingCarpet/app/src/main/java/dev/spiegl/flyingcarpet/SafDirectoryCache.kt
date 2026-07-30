package dev.spiegl.flyingcarpet

import android.content.Context
import android.net.Uri
import android.provider.DocumentsContract
import androidx.documentfile.provider.DocumentFile

// One query per directory, in place of DocumentFile.findFile().
//
// SAF exposes no by-name child lookup — the only primitive is "list this document's
// children" — so findFile() lists the directory and then issues a separate ContentResolver
// query per child to read its display name, because listFiles() keeps only document IDs.
// That is O(children) Binder round trips into com.android.externalstorage.documents, each
// resolving through the FUSE daemon: measured 5.1s for a 219-file folder on a Galaxy A03s
// (1.0s to list the IDs, 3.7s for 219 getName() calls at 16.8ms each). The receive path
// called it twice per file, so a six-photo transfer spent 29 of its 29 seconds here.
//
// Every lookup the receive path needs — does this path exist, how big is it, is "(n) name"
// free — is answered instead from a single cursor per directory carrying document ID,
// display name, size and MIME type together. The result is held for the rest of the
// transfer and updated in place as we create files, so a transfer into one directory costs
// one query no matter how many files it carries.
class SafDirectoryCache(private val context: Context, private val treeUri: Uri) {

    class Entry(
        val documentId: String,
        val name: String,
        val size: Long,
        val isDirectory: Boolean,
    )

    // directory document ID -> its children by display name
    private val directories = HashMap<String, MutableMap<String, Entry>>()

    val rootDocumentId: String = DocumentsContract.getTreeDocumentId(treeUri)

    private fun children(dirDocumentId: String): MutableMap<String, Entry> =
        directories.getOrPut(dirDocumentId) { query(dirDocumentId) }

    private fun query(dirDocumentId: String): MutableMap<String, Entry> {
        val childrenUri =
            DocumentsContract.buildChildDocumentsUriUsingTree(treeUri, dirDocumentId)
        val out = HashMap<String, Entry>()
        context.contentResolver.query(
            childrenUri,
            arrayOf(
                DocumentsContract.Document.COLUMN_DOCUMENT_ID,
                DocumentsContract.Document.COLUMN_DISPLAY_NAME,
                DocumentsContract.Document.COLUMN_SIZE,
                DocumentsContract.Document.COLUMN_MIME_TYPE,
            ),
            null, null, null,
        )?.use { cursor ->
            while (cursor.moveToNext()) {
                val id = cursor.getString(0) ?: continue
                val name = cursor.getString(1) ?: continue
                val size = if (cursor.isNull(2)) 0L else cursor.getLong(2)
                val isDir = cursor.getString(3) == DocumentsContract.Document.MIME_TYPE_DIR
                out[name] = Entry(id, name, size, isDir)
            }
        }
        return out
    }

    fun child(dirDocumentId: String, name: String): Entry? = children(dirDocumentId)[name]

    fun hasChild(dirDocumentId: String, name: String): Boolean =
        children(dirDocumentId).containsKey(name)

    // Resolves a "/"-separated relative path from the tree root. Returns null if any
    // component is missing, or if a non-final component isn't a directory.
    fun resolve(path: String): Entry? {
        val components = path.split('/')
        var dirId = rootDocumentId
        for ((i, component) in components.withIndex()) {
            val entry = child(dirId, component) ?: return null
            if (i == components.size - 1) return entry
            if (!entry.isDirectory) return null
            dirId = entry.documentId
        }
        return null
    }

    // Records something we just created so later lookups in this transfer see it without
    // re-querying. A directory starts with no children rather than being left unqueried:
    // we just created it, so it is known to be empty.
    fun note(dirDocumentId: String, entry: Entry) {
        children(dirDocumentId)[entry.name] = entry
        if (entry.isDirectory) directories.getOrPut(entry.documentId) { HashMap() }
    }

    fun documentUri(documentId: String): Uri =
        DocumentsContract.buildDocumentUriUsingTree(treeUri, documentId)

    private val documentFiles = HashMap<String, DocumentFile>()

    // A DocumentFile for a document inside the tree, used for createFile/createDirectory.
    // fromTreeUri returns a TreeDocumentFile rooted at the document when handed a URI that
    // is both a tree and a document URI, which is what documentUri builds. Memoized because
    // fromTreeUri asks the PackageManager whether the authority is a documents provider on
    // every call, which is the bulk of its cost.
    fun documentFile(documentId: String): DocumentFile? =
        documentFiles.getOrPut(documentId) {
            DocumentFile.fromTreeUri(context, documentUri(documentId)) ?: return null
        }

    // The receive directory itself, for filenames that carry no folders of their own.
    fun rootDocumentFile(): DocumentFile? = documentFile(rootDocumentId)
}

// The document ID of a DocumentFile obtained from this tree. Cheap — it parses the URI
// rather than querying the provider.
fun DocumentFile.treeDocumentId(): String = DocumentsContract.getDocumentId(uri)
