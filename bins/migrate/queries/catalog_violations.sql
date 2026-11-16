WITH context AS (
    SELECT oid, nspname
    FROM pg_catalog.pg_namespace
    WHERE nspname NOT IN ('public', 'information_schema')
        AND nspname NOT LIKE 'pg\_%'
),
group_role AS (
    SELECT oid, rolname
    FROM pg_catalog.pg_roles
    WHERE rolname IN ('app_api', 'app_view', 'app_admin')
),
gate_owner AS (
    SELECT oid FROM pg_catalog.pg_roles WHERE rolname = 'app_view'
),
violation (message) AS (
    SELECT format(
        'table %I.%I: row level security must be enabled and forced',
        n.nspname, c.relname
    )
    FROM pg_catalog.pg_class c
    JOIN context n ON n.oid = c.relnamespace
    WHERE c.relkind IN ('r', 'p')
        AND NOT (c.relrowsecurity AND c.relforcerowsecurity)
    UNION ALL
    SELECT format(
        'policy %I on %I.%I: goes to %s, but a policy goes only to app_api, app_view or app_admin',
        p.polname, n.nspname, c.relname,
        (
            SELECT string_agg(
                CASE WHEN r.role = 0 THEN 'PUBLIC' ELSE pg_catalog.pg_get_userbyid(r.role)::text END,
                ', ' ORDER BY r.role
            )
            FROM unnest(p.polroles) AS r (role)
        )
    )
    FROM pg_catalog.pg_policy p
    JOIN pg_catalog.pg_class c ON c.oid = p.polrelid
    JOIN context n ON n.oid = c.relnamespace
    WHERE EXISTS (
        SELECT FROM unnest(p.polroles) AS r (role)
        WHERE r.role NOT IN (SELECT oid FROM group_role)
    )
    UNION ALL
    SELECT format(
        'view %I.%I: a view without security_invoker is a gate view and belongs to app_view, not %s',
        n.nspname, c.relname, pg_catalog.pg_get_userbyid(c.relowner)
    )
    FROM pg_catalog.pg_class c
    JOIN context n ON n.oid = c.relnamespace
    WHERE c.relkind = 'v'
        AND NOT EXISTS (
            SELECT FROM pg_catalog.pg_options_to_table(c.reloptions) o
            WHERE o.option_name = 'security_invoker'
                AND lower(o.option_value) IN ('true', 'on', 'yes', '1')
        )
        AND c.relowner NOT IN (SELECT oid FROM gate_owner)
    UNION ALL
    SELECT format(
        'function %s: a SECURITY DEFINER function is a gate function and belongs to app_view, not %s',
        p.oid::regprocedure, pg_catalog.pg_get_userbyid(p.proowner)
    )
    FROM pg_catalog.pg_proc p
    JOIN context n ON n.oid = p.pronamespace
    WHERE p.prosecdef
        AND p.proowner NOT IN (SELECT oid FROM gate_owner)
    UNION ALL
    SELECT format(
        'function %s: PUBLIC may execute this SECURITY DEFINER function; REVOKE EXECUTE FROM PUBLIC before OWNER TO app_view',
        p.oid::regprocedure
    )
    FROM pg_catalog.pg_proc p
    JOIN context n ON n.oid = p.pronamespace
    WHERE p.prosecdef
        AND EXISTS (
            SELECT FROM pg_catalog.aclexplode(coalesce(p.proacl, pg_catalog.acldefault('f', p.proowner))) a
            WHERE a.grantee = 0 AND a.privilege_type = 'EXECUTE'
        )
)
SELECT message AS "message!"
FROM violation
ORDER BY message
