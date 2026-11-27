package com.company.odoonativeprint.core

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertThrows
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.io.TempDir
import java.io.File

internal class InMemoryJobPersistence : JobPersistence {
    private val map = LinkedHashMap<String, PrintJobRecord>()
    override fun insert(job: PrintJobRecord) { map[job.id] = job }
    override fun update(job: PrintJobRecord) { map[job.id] = job }
    override fun findById(id: String): PrintJobRecord? = map[id]
    override fun findByIdempotencyKey(key: String): PrintJobRecord? = map.values.firstOrNull { it.idempotencyKey == key }
    override fun listAll(limit: Int): List<PrintJobRecord> = map.values.sortedByDescending { it.createdAtMs }.take(limit)
    override fun countActiveForPrinter(printerName: String): Int = map.values.count { it.printerName == printerName && it.state.isActive }
    override fun countActiveGlobal(): Int = map.values.count { it.state.isActive }
    override fun deleteByIds(ids: List<String>) { ids.forEach { map.remove(it) } }
}

class JobQueueStoreTest {

    @Test
    fun `deduplicates identical idempotency keys`(@TempDir tempDir: File) {
        val engine = JobQueueEngine(InMemoryJobPersistence(), tempDir)
        val first = engine.enqueue("POS-1", "text", "Hello".toByteArray(), idempotencyKey = "pos-order-123")
        val second = engine.enqueue("POS-1", "text", "Hello".toByteArray(), idempotencyKey = "pos-order-123")
        assertEquals(first.id, second.id)
        assertEquals(1, engine.listJobs().size)
    }

    @Test
    fun `enforces per-printer backpressure limit of 500 active jobs`(@TempDir tempDir: File) {
        val engine = JobQueueEngine(InMemoryJobPersistence(), tempDir)
        repeat(PayloadLimits.MAX_ACTIVE_PER_PRINTER) { idx ->
            engine.enqueue("POS-1", "text", "R$idx".toByteArray())
        }
        val ex = assertThrows(QueueCapacityException::class.java) {
            engine.enqueue("POS-1", "text", "Overflow".toByteArray())
        }
        assertEquals("PRINTER_QUEUE_FULL", ex.code)
    }

    @Test
    fun `reconciles interrupted jobs safely on startup`(@TempDir tempDir: File) {
        val engine = JobQueueEngine(InMemoryJobPersistence(), tempDir)
        val prep = engine.enqueue("POS-1", "text", "A".toByteArray())
        val midPrint = engine.enqueue("POS-1", "text", "B".toByteArray())

        engine.transition(prep.id, JobState.RENDERING, incrementAttempt = true)
        engine.transition(midPrint.id, JobState.SUBMITTED, incrementAttempt = true)

        val recovered = engine.reconcileInterruptedJobs(autoReprint = false)
        assertEquals(2, recovered)
        assertEquals(JobState.QUEUED, engine.getJob(prep.id)?.state)
        assertEquals(JobState.FAILED, engine.getJob(midPrint.id)?.state)
        assertEquals("APP_RESTARTED_DURING_PRINT", engine.getJob(midPrint.id)?.errorCode)
    }

    @Test
    fun `prunes failed terminal jobs and never hides older queued jobs`(@TempDir tempDir: File) {
        val engine = JobQueueEngine(InMemoryJobPersistence(), tempDir)
        val queued = engine.enqueue("POS-1", "text", "KEEP".toByteArray())
        repeat(PayloadLimits.MAX_RETAINED_FINISHED + 5) { idx ->
            val j = engine.enqueue("POS-2", "text", "F$idx".toByteArray())
            engine.transition(j.id, JobState.FAILED, errorCode = "OFFLINE")
        }
        assertEquals(queued.id, engine.nextReadyJobForPrinter("POS-1")?.id)
        assertEquals(setOf("POS-1"), engine.printersWithQueuedJobs())
    }
}
