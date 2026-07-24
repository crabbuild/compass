_compass_complete() {
  local current previous
  current="${COMP_WORDS[COMP_CWORD]}"
  previous="${COMP_WORDS[COMP_CWORD-1]}"
  if (( COMP_CWORD == 1 )); then
    COMPREPLY=( $(compgen -W "update extract watch serve cluster-only query path explain affected tree export benchmark diagnose merge-graphs history" -- "$current") )
  elif [[ "${COMP_WORDS[1]}" == "history" && "$COMP_CWORD" == 2 ]]; then
    COMPREPLY=( $(compgen -W "enable disable status build rebuild list show prefer export gc" -- "$current") )
  elif [[ "${COMP_WORDS[1]}" == "history" && "${COMP_WORDS[2]}" == "build" && "$current" == -* ]]; then
    COMPREPLY=( $(compgen -W "--all --first-parent --profile-from --format --code-only --backend --model --mode --cargo --dedup-llm --token-budget --resolution --exclude-hubs --no-gitignore --exclude" -- "$current") )
  elif [[ "$previous" == "export" ]]; then
    COMPREPLY=( $(compgen -W "html callflow-html obsidian wiki svg graphml" -- "$current") )
  elif [[ "$previous" == "diagnose" ]]; then
    COMPREPLY=( $(compgen -W "multigraph" -- "$current") )
  elif [[ "$current" == -* ]]; then
    COMPREPLY=( $(compgen -W "--help --graph --out --output --force --no-cluster --no-viz --no-gitignore --exclude --resolution --exclude-hubs --context --budget --depth --relation --transport --host --port --api-key --path --json-response --stateless --session-timeout" -- "$current") )
  else
    COMPREPLY=( $(compgen -f -- "$current") )
  fi
}
complete -F _compass_complete compass
