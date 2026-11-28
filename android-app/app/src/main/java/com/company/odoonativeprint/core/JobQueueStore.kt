package com.company.odoonativeprint.core

import android.content.ContentValues
import android.content.Context
import android.database.Cursor
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper
import java.io.File
import java.util.UUID

class QueueCapacityException(val code: String, message: String) : IllegalStateException(message)

/**
 * Persistence boundary for JobQueueEngine so the full queue state machine,
 * idempotency deduplication, backpressure, and crash reconciliation can be
 * unit-tested directly on JVM as well as backed by SQLite WAL on Android.
 */
interface JobPersistence {
    fun insert(job: PrintJobRecord)
    fun update(job: PrintJobRecord)
    fun findById(id: String): PrintJobRecord?
    fun findByIdempotencyKey(key: String): PrintJobRecord?
    fun listAll(limit: Int = 500): List<PrintJobRecord>
    fun countActiveForPrinter(printerName: String): Int
    fun countActiveGlobal(): Int
    fun nextPendingRetryAtMs(printerName: String): Long?
    fun deleteByIds(ids: List<String>)
}

class JobQueueEngine(
    private val persistence: JobPersistence,
    private val spoolDir: File,
    private val clock: () -> Long = { System.currentTimeMillis() },
) {
    private val lock = Any()

    init {
        spoolDir.mkdirs()
    }

    fun enqueue(
        printerName: String,
        sourceKind: String,
        payloadBytes: ByteArray,
        copies: Int = 1,
        pages: Int? = null,
        idempotencyKey: String? = null,
        optionsJson: String = "{}",
        maxAttempts: Int = 3,
    ): PrintJobRecord = synchronized(lock) {
        PayloadLimits.validateCopies(copies)
        val cleanPrinter = printerName.trim()
        require(cleanPrinter.isNotEmpty()) { "PRINTER_NAME_EMPTY" }

        if (!idempotencyKey.isNullOrBlank()) {
            val existing = persistence.findByIdempotencyKey(idempotencyKey)
            if (existing != null) return existing
        }

        val printerActive = persistence.countActiveForPrinter(cleanPrinter)
        if (printerActive >= PayloadLimits.MAX_ACTIVE_PER_PRINTER) {
            throw QueueCapacityException(
                "PRINTER_QUEUE_FULL",
                "Active queue limit (${PayloadLimits.MAX_ACTIVE_PER_PRINTER}) reached for $cleanPrinter",
            )
        }
        val globalActive = persistence.countActiveGlobal()
        if (globalActive >= PayloadLimits.MAX_ACTIVE_GLOBAL) {
            throw QueueCapacityException(
                "GLOBAL_QUEUE_FULL",
                "Global active queue limit (${PayloadLimits.MAX_ACTIVE_GLOBAL}) reached",
            )
        }

        val id = UUID.randomUUID().toString()
        val payloadFile = File(spoolDir, "$id.bin")
        payloadFile.writeBytes(payloadBytes)

        val now = clock()
        val record = PrintJobRecord(
            id = id,
            idempotencyKey = idempotencyKey?.takeIf { it.isNotBlank() },
            printerName = cleanPrinter,
            sourceKind = sourceKind,
            state = JobState.QUEUED,
            attempts = 0,
            maxAttempts = maxAttempts.coerceIn(1, 5),
            pages = pages,
            copies = copies,
            optionsJson = optionsJson,
            payloadPath = payloadFile.absolutePath,
            createdAtMs = now,
            updatedAtMs = now,
        )
        persistence.insert(record)
        return record
    }

    fun getJob(id: String): PrintJobRecord? = synchronized(lock) {
        persistence.findById(id) ?: persistence.findByIdempotencyKey(id)
    }

    fun listJobs(limit: Int = 200): List<PrintJobRecord> = synchronized(lock) { persistence.listAll(limit) }

    fun summary(): QueueSummary = synchronized(lock) {
        val all = persistence.listAll(PayloadLimits.MAX_ACTIVE_GLOBAL + PayloadLimits.MAX_RETAINED_FINISHED)
        QueueSummary(
            active = all.count { it.state.isActive },
            queued = all.count { it.state == JobState.QUEUED },
            failed = all.count { it.state == JobState.FAILED },
            completed = all.count { it.state == JobState.COMPLETED },
        )
    }

    fun nextReadyJobForPrinter(printerName: String): PrintJobRecord? = synchronized(lock) {
        val now = clock()
        persistence.listAll(PayloadLimits.MAX_ACTIVE_GLOBAL + PayloadLimits.MAX_RETAINED_FINISHED)
            .filter { it.printerName == printerName && it.state == JobState.QUEUED && it.nextRetryAtMs <= now }
            .minByOrNull { it.createdAtMs }
    }

    fun nextPendingRetryAtMs(printerName: String): Long? = synchronized(lock) {
        persistence.nextPendingRetryAtMs(printerName)
    }

    fun printersWithQueuedJobs(): Set<String> = synchronized(lock) {
        persistence.listAll(PayloadLimits.MAX_ACTIVE_GLOBAL + PayloadLimits.MAX_RETAINED_FINISHED)
            .filter { it.state == JobState.QUEUED }
            .map { it.printerName }
            .toSet()
    }

    fun transition(
        id: String,
        newState: JobState,
        incrementAttempt: Boolean = false,
        errorCode: String? = null,
        errorMessage: String? = null,
        nextRetryAtMs: Long = 0L,
    ): PrintJobRecord? = synchronized(lock) {
        val current = persistence.findById(id) ?: return null
        if (current.state.isTerminal) return current
        val updated = current.copy(
            state = newState,
            attempts = if (incrementAttempt) current.attempts + 1 else current.attempts,
            errorCode = errorCode,
            errorMessage = errorMessage,
            updatedAtMs = clock(),
            nextRetryAtMs = nextRetryAtMs,
        )
        persistence.update(updated)
        if (newState == JobState.COMPLETED) {
            // Keep spool file only if failed (so user can retry); completed jobs free disk immediately.
            File(updated.payloadPath).delete()
        }
        if (newState.isTerminal) {
            pruneFinishedLocked()
        }
        return updated
    }

    fun retryJob(id: String): PrintJobRecord? = synchronized(lock) {
        val current = persistence.findById(id) ?: return null
        if (current.state != JobState.FAILED && current.state != JobState.CANCELLED) return null
        if (!File(current.payloadPath).exists()) return null
        val updated = current.copy(
            state = JobState.QUEUED,
            attempts = 0,
            errorCode = null,
            errorMessage = null,
            nextRetryAtMs = 0L,
            updatedAtMs = clock(),
        )
        persistence.update(updated)
        return updated
    }

    fun cancelJob(id: String): PrintJobRecord? = synchronized(lock) {
        val current = persistence.findById(id) ?: return null
        if (current.state.isTerminal) return current
        val updated = current.copy(
            state = JobState.CANCELLED,
            errorCode = "USER_CANCELLED",
            errorMessage = "Cancelled by user",
            updatedAtMs = clock(),
        )
        persistence.update(updated)
        File(updated.payloadPath).delete()
        pruneFinishedLocked()
        return updated
    }

    fun clearFinished(): Int = synchronized(lock) {
        val terminal = persistence.listAll(5_000).filter { it.state.isTerminal }
        terminal.forEach { File(it.payloadPath).delete() }
        persistence.deleteByIds(terminal.map { it.id })
        terminal.size
    }

    /**
     * Startup crash recovery:
     * - RENDERING / SPOOLING never touched the printer -> safe to requeue if attempts remain.
     * - SUBMITTED / IN_PROGRESS may have already printed -> mark FAILED unless autoReprint is enabled.
     */
    fun reconcileInterruptedJobs(autoReprint: Boolean): Int = synchronized(lock) {
        val now = clock()
        var recovered = 0
        for (job in persistence.listAll(PayloadLimits.MAX_ACTIVE_GLOBAL + PayloadLimits.MAX_RETAINED_FINISHED)) {
            when (job.state) {
                JobState.RENDERING, JobState.SPOOLING -> {
                    val next = if (job.attempts < job.maxAttempts && File(job.payloadPath).exists()) {
                        job.copy(state = JobState.QUEUED, nextRetryAtMs = 0L, updatedAtMs = now)
                    } else {
                        job.copy(
                            state = JobState.FAILED,
                            errorCode = "APP_RESTARTED_DURING_PREP",
                            errorMessage = "App restarted before spooling completed",
                            updatedAtMs = now,
                        )
                    }
                    persistence.update(next)
                    recovered++
                }
                JobState.SUBMITTED, JobState.IN_PROGRESS -> {
                    val next = if (autoReprint && File(job.payloadPath).exists()) {
                        job.copy(state = JobState.QUEUED, nextRetryAtMs = 0L, updatedAtMs = now)
                    } else {
                        job.copy(
                            state = JobState.FAILED,
                            errorCode = "APP_RESTARTED_DURING_PRINT",
                            errorMessage = "App restarted while printing; retry manually if needed",
                            updatedAtMs = now,
                        )
                    }
                    persistence.update(next)
                    recovered++
                }
                else -> Unit
            }
        }
        return recovered
    }

    private fun pruneFinishedLocked() {
        val finished = persistence.listAll(PayloadLimits.MAX_ACTIVE_GLOBAL + PayloadLimits.MAX_RETAINED_FINISHED)
            .filter { it.state.isTerminal }
            .sortedByDescending { it.updatedAtMs }
        if (finished.size > PayloadLimits.MAX_RETAINED_FINISHED) {
            val drop = finished.drop(PayloadLimits.MAX_RETAINED_FINISHED)
            drop.forEach { File(it.payloadPath).delete() }
            persistence.deleteByIds(drop.map { it.id })
        }
    }
}

/**
 * Durable SQLite WAL implementation for Android production runtime.
 */
class SqliteJobPersistence(context: Context) :
    SQLiteOpenHelper(context, DB_NAME, null, DB_VERSION), JobPersistence {

    init {
        setWriteAheadLoggingEnabled(true)
    }

    override fun onConfigure(db: SQLiteDatabase) {
        super.onConfigure(db)
        db.enableWriteAheadLogging()
        db.execSQL("PRAGMA synchronous=NORMAL;")
        db.rawQuery("PRAGMA busy_timeout=5000;", null).use { it.moveToFirst() }
    }

    override fun onCreate(db: SQLiteDatabase) {
        db.execSQL(
            """
            CREATE TABLE print_jobs (
                id TEXT PRIMARY KEY,
                idempotency_key TEXT UNIQUE,
                printer_name TEXT NOT NULL,
                source_kind TEXT NOT NULL,
                state TEXT NOT NULL,
                attempts INTEGER NOT NULL DEFAULT 0,
                max_attempts INTEGER NOT NULL DEFAULT 3,
                pages INTEGER,
                copies INTEGER NOT NULL DEFAULT 1,
                options_json TEXT NOT NULL,
                payload_path TEXT NOT NULL,
                error_code TEXT,
                error_message TEXT,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL,
                next_retry_at_ms INTEGER NOT NULL DEFAULT 0
            )
            """.trimIndent(),
        )
        db.execSQL("CREATE INDEX idx_jobs_printer_state ON print_jobs(printer_name, state)")
        db.execSQL("CREATE INDEX idx_jobs_state_retry ON print_jobs(state, next_retry_at_ms)")
        db.execSQL("CREATE INDEX idx_jobs_created ON print_jobs(created_at_ms DESC)")
    }

    override fun onUpgrade(db: SQLiteDatabase, oldVersion: Int, newVersion: Int) = Unit

    override fun insert(job: PrintJobRecord) {
        writableDatabase.insertOrThrow(TABLE, null, job.toContentValues())
    }

    override fun update(job: PrintJobRecord) {
        writableDatabase.update(TABLE, job.toContentValues(), "id = ?", arrayOf(job.id))
    }

    override fun findById(id: String): PrintJobRecord? =
        readableDatabase.query(TABLE, null, "id = ?", arrayOf(id), null, null, null).use { c ->
            if (c.moveToFirst()) c.toJobRecord() else null
        }

    override fun findByIdempotencyKey(key: String): PrintJobRecord? =
        readableDatabase.query(TABLE, null, "idempotency_key = ?", arrayOf(key), null, null, null).use { c ->
            if (c.moveToFirst()) c.toJobRecord() else null
        }

    override fun listAll(limit: Int): List<PrintJobRecord> =
        readableDatabase.query(TABLE, null, null, null, null, null, "created_at_ms DESC", limit.toString()).use { c ->
            val out = ArrayList<PrintJobRecord>(c.count)
            while (c.moveToNext()) out += c.toJobRecord()
            out
        }

    override fun countActiveForPrinter(printerName: String): Int {
        val sql = "SELECT COUNT(*) FROM $TABLE WHERE printer_name = ? AND state IN ('queued','rendering','spooling','submitted','in_progress')"
        return readableDatabase.rawQuery(sql, arrayOf(printerName)).use { c ->
            if (c.moveToFirst()) c.getInt(0) else 0
        }
    }

    override fun countActiveGlobal(): Int {
        val sql = "SELECT COUNT(*) FROM $TABLE WHERE state IN ('queued','rendering','spooling','submitted','in_progress')"
        return readableDatabase.rawQuery(sql, null).use { c ->
            if (c.moveToFirst()) c.getInt(0) else 0
        }
    }

    override fun nextPendingRetryAtMs(printerName: String): Long? {
        val sql = "SELECT MIN(next_retry_at_ms) FROM $TABLE WHERE printer_name = ? AND state = 'queued'"
        return readableDatabase.rawQuery(sql, arrayOf(printerName)).use { c ->
            if (c.moveToFirst() && !c.isNull(0)) c.getLong(0) else null
        }
    }

    override fun deleteByIds(ids: List<String>) {
        if (ids.isEmpty()) return
        val db = writableDatabase
        // WAL mode: BEGIN IMMEDIATE allows concurrent readers while pruning finished rows.
        db.beginTransactionNonExclusive()
        try {
            ids.chunked(200).forEach { chunk ->
                val placeholders = chunk.joinToString(",") { "?" }
                db.delete(TABLE, "id IN ($placeholders)", chunk.toTypedArray())
            }
            db.setTransactionSuccessful()
        } finally {
            db.endTransaction()
        }
    }

    private fun PrintJobRecord.toContentValues(): ContentValues = ContentValues().apply {
        put("id", id)
        put("idempotency_key", idempotencyKey)
        put("printer_name", printerName)
        put("source_kind", sourceKind)
        put("state", state.wireName)
        put("attempts", attempts)
        put("max_attempts", maxAttempts)
        if (pages != null) put("pages", pages) else putNull("pages")
        put("copies", copies)
        put("options_json", optionsJson)
        put("payload_path", payloadPath)
        put("error_code", errorCode)
        put("error_message", errorMessage)
        put("created_at_ms", createdAtMs)
        put("updated_at_ms", updatedAtMs)
        put("next_retry_at_ms", nextRetryAtMs)
    }

    private fun Cursor.toJobRecord(): PrintJobRecord = PrintJobRecord(
        id = getString(getColumnIndexOrThrow("id")),
        idempotencyKey = getString(getColumnIndexOrThrow("idempotency_key")),
        printerName = getString(getColumnIndexOrThrow("printer_name")),
        sourceKind = getString(getColumnIndexOrThrow("source_kind")),
        state = JobState.fromWire(getString(getColumnIndexOrThrow("state"))),
        attempts = getInt(getColumnIndexOrThrow("attempts")),
        maxAttempts = getInt(getColumnIndexOrThrow("max_attempts")),
        pages = getColumnIndexOrThrow("pages").let { idx -> if (isNull(idx)) null else getInt(idx) },
        copies = getInt(getColumnIndexOrThrow("copies")),
        optionsJson = getString(getColumnIndexOrThrow("options_json")),
        payloadPath = getString(getColumnIndexOrThrow("payload_path")),
        errorCode = getString(getColumnIndexOrThrow("error_code")),
        errorMessage = getString(getColumnIndexOrThrow("error_message")),
        createdAtMs = getLong(getColumnIndexOrThrow("created_at_ms")),
        updatedAtMs = getLong(getColumnIndexOrThrow("updated_at_ms")),
        nextRetryAtMs = getLong(getColumnIndexOrThrow("next_retry_at_ms")),
    )

    companion object {
        private const val DB_NAME = "itbaa_queue.db"
        private const val DB_VERSION = 1
        private const val TABLE = "print_jobs"
    }
}
