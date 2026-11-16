INSERT INTO catalog.item (organization_id, title)
VALUES ($1, $2)
RETURNING id, organization_id, title
