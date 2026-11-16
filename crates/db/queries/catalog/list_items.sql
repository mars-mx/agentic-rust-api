SELECT id AS "id!", organization_id, title AS "title!"
FROM catalog.visible_item
ORDER BY title, id
LIMIT 100
