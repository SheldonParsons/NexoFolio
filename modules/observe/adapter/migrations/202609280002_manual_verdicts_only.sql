-- Addresses are the project's own unless someone decides otherwise (0003 §4.2):
-- no automatic verdicts, so nothing looks across projects either.
DROP INDEX service_addresses_address;
ALTER TABLE service_addresses DROP COLUMN auto_verdict, DROP COLUMN auto_reason;
