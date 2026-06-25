package org.streamflow.bridge;

/** Thrown when a job is cooperatively cancelled and the engine confirms it. */
public class RJobCancelledException extends RuntimeException {
    public RJobCancelledException(long id) {
        super("R job " + id + " was cancelled");
    }
}
