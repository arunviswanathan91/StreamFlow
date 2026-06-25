package org.streamflow.bridge;

/** A progress event emitted by the R engine during a long job. */
public record RProgress(long id, double frac, String message) {}
