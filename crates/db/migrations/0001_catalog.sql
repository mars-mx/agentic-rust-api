CREATE SCHEMA catalog;

CREATE TABLE catalog.item (
    id uuid PRIMARY KEY DEFAULT uuidv7(),
    organization_id text,
    title text NOT NULL,
    created_by text NOT NULL DEFAULT current_setting('app.user_id', true),
    search tsvector GENERATED ALWAYS AS (to_tsvector('simple', title)) STORED
);
CREATE INDEX item_organization ON catalog.item (organization_id);
CREATE INDEX item_search ON catalog.item USING gin (search);

ALTER TABLE catalog.item ENABLE ROW LEVEL SECURITY;
ALTER TABLE catalog.item FORCE ROW LEVEL SECURITY;

CREATE POLICY org_scope ON catalog.item TO app_api
    USING (organization_id = current_setting('app.organization_id'))
    WITH CHECK (organization_id = current_setting('app.organization_id'));
CREATE POLICY view_read ON catalog.item FOR SELECT TO app_view
    USING (true);
CREATE POLICY admin_access ON catalog.item TO app_admin
    USING (true)
    WITH CHECK (true);

GRANT USAGE ON SCHEMA catalog TO app_api, app_view, app_admin;
GRANT SELECT, INSERT, UPDATE, DELETE ON catalog.item TO app_api, app_admin;
GRANT SELECT ON catalog.item TO app_view;

CREATE VIEW catalog.visible_item AS
    SELECT i.id, i.organization_id, i.title
    FROM catalog.item i
    WHERE i.organization_id IS NULL OR i.organization_id = current_setting('app.organization_id');
GRANT SELECT ON catalog.visible_item TO app_api;
ALTER VIEW catalog.visible_item OWNER TO app_view;

CREATE FUNCTION catalog.search_items(term text)
    RETURNS TABLE (id uuid, organization_id text, title text)
    LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, pg_temp
    AS $$
        SELECT i.id, i.organization_id, i.title
        FROM catalog.item i
        WHERE i.search @@ websearch_to_tsquery('simple', term)
            AND (i.organization_id IS NULL OR i.organization_id = current_setting('app.organization_id'))
    $$;
REVOKE EXECUTE ON FUNCTION catalog.search_items(text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION catalog.search_items(text) TO app_api;
ALTER FUNCTION catalog.search_items(text) OWNER TO app_view;
