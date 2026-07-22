MATCH p=(a:Function)-[:CALLS*1..4]->(b)
WHERE all(r IN relationships(p) WHERE r.confidence = 'EXTRACTED')
RETURN a.id, b.id, length(p) AS hops ORDER BY hops LIMIT 100
