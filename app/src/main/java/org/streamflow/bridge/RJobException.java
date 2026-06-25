package org.streamflow.bridge;

import java.util.List;

/**
 * Thrown when the R engine reports an {@code error} reply. Carries the R-side
 * message and traceback so the UI can show a real, supportable diagnostic
 * rather than a generic failure.
 */
public class RJobException extends RuntimeException {

    private final List<String> trace;

    public RJobException(String message, List<String> trace) {
        super(message);
        this.trace = trace == null ? List.of() : List.copyOf(trace);
    }

    /** R traceback lines (most recent first), possibly empty. */
    public List<String> trace() {
        return trace;
    }

    @Override
    public String toString() {
        return trace.isEmpty()
                ? "RJobException: " + getMessage()
                : "RJobException: " + getMessage() + "\n  at " + String.join("\n  at ", trace);
    }
}
