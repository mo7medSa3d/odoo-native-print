package com.company.odoonativeprint.core

/**
 * Payload and queue backpressure limits matching desktop crates/print-core.
 */
object PayloadLimits {
    const val MAX_PDF_BYTES: Int = 50 * 1024 * 1024
    const val MAX_PDF_PAGES: Int = 2_000
    const val MIN_COPIES: Int = 1
    const val MAX_COPIES: Int = 99

    const val MAX_IMAGE_BYTES: Int = 20 * 1024 * 1024
    const val MAX_IMAGE_SIDE: Int = 40_000
    const val MAX_IMAGE_PIXELS: Long = 50_000_000L

    const val MAX_TEXT_BYTES: Int = 1 * 1024 * 1024

    const val MAX_ACTIVE_PER_PRINTER: Int = 500
    const val MAX_ACTIVE_GLOBAL: Int = 2_000
    const val MAX_RETAINED_FINISHED: Int = 500

    fun validateCopies(copies: Int): Int {
        require(copies in MIN_COPIES..MAX_COPIES) {
            "COPIES_OUT_OF_RANGE: must be $MIN_COPIES..$MAX_COPIES"
        }
        return copies
    }
}
