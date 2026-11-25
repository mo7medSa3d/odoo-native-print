package com.company.odoonativeprint.worker

import com.company.odoonativeprint.core.JobQueueEngine
import com.company.odoonativeprint.core.JobState
import com.company.odoonativeprint.core.PrintJobRecord
import com.company.odoonativeprint.core.QueueSummary
import com.company.odoonativeprint.transport.TransportResult
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import java.util.concurrent.ConcurrentHashMap

fun interface JobExecutor {
    suspend fun execute(job: PrintJobRecord): TransportResult
}

/**
 * Per-printer serialized worker coordinator:
 * - Jobs for the SAME printer run strictly FIFO (never interleaved on TCP/BT/USB).
 * - Jobs for DIFFERENT printers run concurrently.
 * - Transient failures back off (2s, 6s, 15s) up to maxAttempts; non-retryable errors fail immediately.
 */
class PrintWorkerCoordinator(
    private val queue: JobQueueEngine,
    private val executor: JobExecutor,
    private val onActiveCountChanged: (Int) -> Unit = {},
    private val dispatcher: CoroutineDispatcher = Dispatchers.IO,
    private val backoffScheduleMs: LongArray = longArrayOf(2_000L, 6_000L, 15_000L),
) {
    private val scope = CoroutineScope(SupervisorJob() + dispatcher)
    private val printerWorkers = ConcurrentHashMap<String, Job>()

    private val _summary = MutableStateFlow(queue.summary())
    val summary: StateFlow<QueueSummary> = _summary.asStateFlow()

    fun start(autoReprint: Boolean) {
        queue.reconcileInterruptedJobs(autoReprint)
        refreshSummary()
        queue.printersWithQueuedJobs().forEach { wakePrinter(it) }
    }

    fun wakePrinter(printerName: String) {
        refreshSummary()
        printerWorkers.compute(printerName) { _, existing ->
            if (existing != null && existing.isActive) {
                existing
            } else {
                scope.launch { runPrinterLoop(printerName) }
            }
        }
    }

    fun refreshSummary() {
        val s = queue.summary()
        _summary.value = s
        onActiveCountChanged(s.active)
    }

    private suspend fun runPrinterLoop(printerName: String) {
        try {
            while (scope.isActive) {
                val job = queue.nextReadyJobForPrinter(printerName)
                if (job == null) {
                    // Check if any delayed retry is pending for this printer
                    val pending = queue.listJobs(500)
                        .filter { it.printerName == printerName && it.state == JobState.QUEUED }
                        .minByOrNull { it.nextRetryAtMs }
                    if (pending == null) break
                    val waitMs = (pending.nextRetryAtMs - System.currentTimeMillis()).coerceIn(50L, 5_000L)
                    delay(waitMs)
                    continue
                }

                processSingleJob(job)
                refreshSummary()
            }
        } finally {
            val self = currentCoroutineContext()[Job]
            if (self != null) printerWorkers.remove(printerName, self) else printerWorkers.remove(printerName)
            refreshSummary()
            if (scope.isActive && queue.nextReadyJobForPrinter(printerName) != null) {
                wakePrinter(printerName)
            }
        }
    }

    private suspend fun processSingleJob(job: PrintJobRecord) {
        val rendering = queue.transition(job.id, JobState.RENDERING, incrementAttempt = true) ?: return
        if (rendering.state == JobState.CANCELLED) return

        queue.transition(job.id, JobState.SUBMITTED)
        val result = try {
            executor.execute(rendering)
        } catch (e: Exception) {
            TransportResult.Failure(
                errorCode = "WORKER_EXCEPTION",
                message = e.message ?: e.javaClass.simpleName,
                retryable = true,
            )
        }

        // Respect mid-flight cancellation
        if (queue.getJob(job.id)?.state == JobState.CANCELLED) return

        when (result) {
            is TransportResult.Ok -> {
                queue.transition(job.id, JobState.COMPLETED)
            }
            is TransportResult.Failure -> {
                val attemptsUsed = rendering.attempts
                if (result.retryable && attemptsUsed < rendering.maxAttempts) {
                    val backoff = backoffScheduleMs.getOrElse(attemptsUsed - 1) { backoffScheduleMs.last() }
                    queue.transition(
                        id = job.id,
                        newState = JobState.QUEUED,
                        errorCode = result.errorCode,
                        errorMessage = result.message,
                        nextRetryAtMs = System.currentTimeMillis() + backoff,
                    )
                } else {
                    queue.transition(
                        id = job.id,
                        newState = JobState.FAILED,
                        errorCode = result.errorCode,
                        errorMessage = result.message,
                    )
                }
            }
        }
    }
}
