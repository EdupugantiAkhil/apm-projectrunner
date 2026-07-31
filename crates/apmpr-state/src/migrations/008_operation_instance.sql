ALTER TABLE operations ADD COLUMN instance TEXT;

CREATE INDEX operations_instance_started ON operations(instance, started_at DESC, id DESC);
