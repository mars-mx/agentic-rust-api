ALTER TABLE catalog.item NO FORCE ROW LEVEL SECURITY;
CREATE TABLE catalog.open (id uuid PRIMARY KEY);
CREATE POLICY everyone ON catalog.item FOR SELECT TO PUBLIC USING (true);
CREATE POLICY login ON catalog.item FOR SELECT TO app_api, postgres USING (true);
ALTER VIEW catalog.visible_item OWNER TO postgres;
CREATE VIEW catalog.invoker_view WITH (security_invoker = true) AS SELECT id FROM catalog.item;
CREATE FUNCTION catalog.leak() RETURNS SETOF uuid LANGUAGE sql SECURITY DEFINER AS $$ SELECT id FROM catalog.item $$;
