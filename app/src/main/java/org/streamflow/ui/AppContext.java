package org.streamflow.ui;

import org.streamflow.bridge.BridgeService;

/** Shared services handed to each module controller once the engine is ready. */
public record AppContext(BridgeService bridge, JobRunner jobs, ChannelAliases aliases,
                         WorkspaceModel workspace, AppSettings settings, AuditLog auditLog,
                         FmoStore fmo) {}
