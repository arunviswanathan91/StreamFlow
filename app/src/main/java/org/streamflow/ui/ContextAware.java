package org.streamflow.ui;

/** A module controller that receives shared services once the engine is ready. */
public interface ContextAware {
    void init(AppContext context);
}
