-- NexTerm schema v3 (Phase 6): per-session switch for the live resource bar
-- (CPU / RAM / disk read over a separate SSH exec channel). On by default.
ALTER TABLE sessions ADD COLUMN resource_monitor INTEGER NOT NULL DEFAULT 1;
