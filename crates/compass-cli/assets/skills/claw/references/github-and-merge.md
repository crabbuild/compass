# graphify reference: GitHub clone and cross-repo merge

Load this when the user passed one or more `https://github.com/...` URLs, or named several local subfolders to merge into one graph.

### Step 0 - Clone GitHub repo(s) (only if a GitHub URL was given)

**Single repo:**
```bash
LOCAL_PATH=$(graphify clone <github-url> [--branch <branch>])
# Use LOCAL_PATH as the target for all subsequent steps
```

**Multiple repos (cross-repo graph):**
```bash
# Clone each repo, run the full pipeline on each, then merge
graphify clone <url1>   # → ~/.graphify/repos/<owner1>/<repo1>
graphify clone <url2>   # → ~/.graphify/repos/<owner2>/<repo2>
# Run /compass on each local path to produce their graph.json files
# Then merge:
graphify merge-graphs \
  ~/.graphify/repos/<owner1>/<repo1>/compass-out/graph.json \
  ~/.graphify/repos/<owner2>/<repo2>/compass-out/graph.json \
  --out compass-out/cross-repo-graph.json
```

Graphify clones into `~/.graphify/repos/<owner>/<repo>` and reuses existing clones on repeat runs. Each node in the merged graph carries a `repo` attribute so you can filter by origin.

**Multiple local subfolders (monorepo or multi-service layout):**

The skill pipeline writes all intermediate and final outputs to `compass-out/` in the current working directory. Running the skill on each subfolder separately will clobber the same output dir. Instead, use the CLI directly for each subfolder — it places `compass-out/` *inside* the scanned path:

```bash
compass extract ./core/     # → ./core/compass-out/graph.json
compass extract ./service/  # → ./service/compass-out/graph.json
compass extract ./platform/ # → ./platform/compass-out/graph.json
# Add --backend gemini|kimi|openai|deepseek|claude-cli depending on which API key you have set

# Then merge at the project root:
graphify merge-graphs \
  ./core/compass-out/graph.json \
  ./service/compass-out/graph.json \
  ./platform/compass-out/graph.json \
  --out compass-out/graph.json
```

Once `compass-out/graph.json` exists, the fast path above takes over: any codebase question runs `compass query` directly on the merged graph — no re-extraction, no size gate.
