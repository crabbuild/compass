Return exactly one JSON object matching the reviewer mandate. It must contain a
`findings` array. Each finding must use severity `CRITICAL`, `WARNING`, or
`SUGGESTION` and include non-empty `claim` and `evidence` strings. If there are
no findings, return an empty `findings` array plus the required `checked_classes`
due-diligence list. Do not emit prose, Markdown fences, or a truncated object.
