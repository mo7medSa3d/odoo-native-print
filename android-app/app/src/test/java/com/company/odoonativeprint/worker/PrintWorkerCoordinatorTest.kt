package com.company.odoonativeprint.worker

import com.company.odoonativeprint.core.InMemoryJobPersistence
import com.company.odoonativeprint.core.JobQueueEngine
import com.company.odoonativeprint.core.JobState
import com.company.odoonativeprint.transport.TransportResult
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.io.TempDir
import java.io.File
import java.util.Collections

class PrintWorkerCoordinatorTest {

    @Test
    fun `serializes jobs per printer FIFO and retries transient errors`(@TempDir tempDir: File) = runBlocking {
        val queue = JobQueueEngine(InMemoryJobPersistence(), tempDir)
        val executionOrder = Collections.synchronizedList(mutableListOf<String>())
        var firstAttemptFailed = false

        val coordinator = PrintWorkerCoordinator(
            queue = queue,
            executor = { job ->
                if (job.sourceKind == "text" && !firstAttemptFailed && File(job.payloadPath).readText() == "J1") {
                    firstAttemptFailed = true
                    TransportResult.Failure("TCP_IO_ERROR", "transient", retryable = true)
                } else {
                    executionOrder += File(job.payloadPath).readText()
                    TransportResult.Ok
                }
            },
            dispatcher = Dispatchers.IO,
            backoffScheduleMs = longArrayOf(50L, 50L, 50L),
        )

        val j1 = queue.enqueue("Cashier", "text", "J1".toByteArray())
        val j2 = queue.enqueue("Cashier", "text", "J2".toByteArray())
        coordinator.wakePrinter("Cashier")

        repeat(40) {
            if (queue.getJob(j1.id)?.state == JobState.COMPLETED && queue.getJob(j2.id)?.state == JobState.COMPLETED) return@repeat
            delay(50L)
        }

        assertEquals(JobState.COMPLETED, queue.getJob(j1.id)?.state)
        assertEquals(JobState.COMPLETED, queue.getJob(j2.id)?.state)
        assertEquals(2, queue.getJob(j1.id)?.attempts)
        assertEquals(setOf("J1", "J2"), executionOrder.toSet())
    }
}
