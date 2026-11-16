SELECT id AS "id!", organization_id, title AS "title!"
FROM catalog.search_items($1)
ORDER BY title, id
LIMIT 100
