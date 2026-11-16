CREATE SCHEMA fixture;

CREATE TABLE fixture.open (id uuid PRIMARY KEY);
CREATE TABLE fixture.unforced (id uuid PRIMARY KEY);
ALTER TABLE fixture.unforced ENABLE ROW LEVEL SECURITY;

CREATE TABLE fixture.guarded (id uuid PRIMARY KEY, organization_id text);
ALTER TABLE fixture.guarded ENABLE ROW LEVEL SECURITY;
ALTER TABLE fixture.guarded FORCE ROW LEVEL SECURITY;
CREATE POLICY view_read ON fixture.guarded FOR SELECT TO app_view USING (true);
CREATE POLICY everyone ON fixture.guarded FOR SELECT TO PUBLIC USING (true);
CREATE POLICY login ON fixture.guarded FOR SELECT TO app_api, postgres USING (true);

CREATE VIEW fixture.gate AS SELECT id FROM fixture.guarded;
ALTER VIEW fixture.gate OWNER TO app_view;
CREATE VIEW fixture.invoker_view WITH (security_invoker = true) AS SELECT id FROM fixture.guarded;
CREATE VIEW fixture.unowned_gate AS SELECT id FROM fixture.guarded;

CREATE FUNCTION fixture.closed_gate() RETURNS SETOF uuid LANGUAGE sql SECURITY DEFINER AS $$ SELECT id FROM fixture.guarded $$;
REVOKE EXECUTE ON FUNCTION fixture.closed_gate() FROM PUBLIC;
ALTER FUNCTION fixture.closed_gate() OWNER TO app_view;
CREATE FUNCTION fixture.leak() RETURNS SETOF uuid LANGUAGE sql SECURITY DEFINER AS $$ SELECT id FROM fixture.guarded $$;
