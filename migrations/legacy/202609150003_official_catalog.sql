-- A project always has a formal view. Missing assignments belong to its immutable system folder.
CREATE TABLE project_catalogs (
 project_id uuid PRIMARY KEY REFERENCES projects(id),
 unclassified_id uuid NOT NULL UNIQUE DEFAULT gen_random_uuid(),
 current_version_id uuid,
 generation bigint NOT NULL DEFAULT 0 CHECK(generation>=0)
);
CREATE TABLE catalog_versions (
 id uuid PRIMARY KEY,
 project_id uuid NOT NULL REFERENCES projects(id),
 source_task_id uuid NOT NULL UNIQUE REFERENCES catalog_preview_tasks(id),
 candidate jsonb NOT NULL,
 created_by uuid NOT NULL REFERENCES users(id),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 UNIQUE(project_id,id)
);
ALTER TABLE project_catalogs ADD FOREIGN KEY(project_id,current_version_id) REFERENCES catalog_versions(project_id,id);
CREATE TABLE catalog_version_nodes (
 version_id uuid NOT NULL REFERENCES catalog_versions(id),
 id uuid NOT NULL,parent_id uuid,name text NOT NULL CHECK(name<>'待分类'),description text NOT NULL,
 PRIMARY KEY(version_id,id),
 FOREIGN KEY(version_id,parent_id) REFERENCES catalog_version_nodes(version_id,id) DEFERRABLE INITIALLY DEFERRED
);
CREATE TABLE catalog_version_assignments (
 version_id uuid NOT NULL REFERENCES catalog_versions(id),interface_id uuid NOT NULL REFERENCES interface_documents(id),directory_id uuid,
 PRIMARY KEY(version_id,interface_id),
 FOREIGN KEY(version_id,directory_id) REFERENCES catalog_version_nodes(version_id,id) DEFERRABLE INITIALLY DEFERRED
);
CREATE TABLE catalog_activation_receipts (
 project_id uuid NOT NULL REFERENCES projects(id),request_id uuid NOT NULL,actor_id uuid NOT NULL REFERENCES users(id),
 command jsonb NOT NULL,result jsonb NOT NULL,created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(project_id,request_id)
);
CREATE FUNCTION initialize_project_catalog() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN INSERT INTO project_catalogs(project_id) VALUES(NEW.id); RETURN NEW; END $$;
CREATE TRIGGER initialize_project_catalog AFTER INSERT ON projects FOR EACH ROW EXECUTE FUNCTION initialize_project_catalog();
INSERT INTO project_catalogs(project_id) SELECT id FROM projects;
CREATE FUNCTION protect_system_directory() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP='DELETE' THEN RAISE EXCEPTION 'system directory cannot be deleted'; END IF;
 IF NEW.project_id<>OLD.project_id OR NEW.unclassified_id<>OLD.unclassified_id THEN RAISE EXCEPTION 'system directory identity is immutable'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER protect_system_directory BEFORE UPDATE OR DELETE ON project_catalogs FOR EACH ROW EXECUTE FUNCTION protect_system_directory();
