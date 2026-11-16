SELECT
FROM
    set_config('app.organization_id', $1, true) AS organization,
    set_config('app.user_id', $2, true) AS member
