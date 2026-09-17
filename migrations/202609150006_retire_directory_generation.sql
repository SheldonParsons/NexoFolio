-- Retire only unfinished work owned by the removed directory generator.
-- Completed candidates, snapshots and published versions remain unchanged.
UPDATE catalog_preview_tasks
SET status='failed', error_code='LEGACY_GENERATION_RETIRED',
    generation=generation+1, lease_until=NULL, finished_at=clock_timestamp()
WHERE status IN ('pending','running');
