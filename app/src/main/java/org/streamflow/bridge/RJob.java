package org.streamflow.bridge;

import com.fasterxml.jackson.databind.JsonNode;

import java.util.concurrent.Future;

/** Handle to a submitted engine job: its protocol id plus the pending result. */
public record RJob(long id, Future<JsonNode> future) {}
