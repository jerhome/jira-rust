#!/usr/bin/env bash
# Empêche Claude et ses agents de joindre une instance Jira réelle sans autorisation de l'utilisateur.
# Protection « best effort » par analyse du texte des commandes ; la vraie barrière est le garde-fou
# du code (crates/jira-client/src/guard.rs + .cargo/config.toml).
# Pour une connexion autorisée, l'utilisateur la lance lui-même avec `! <commande>` (non soumis aux hooks).
set -uo pipefail

deny() {
  echo "Bloqué : $1 Interdit sans autorisation explicite de l'utilisateur (règle du projet jira-rust). Utiliser wiremock / MockJira / --mock." >&2
  exit 2
}

# Exception phase 3 (accord utilisateur 2026-11-11) : jira-rust sans --mock est permis seulement si
# un unique --config désigne un fichier absolu du scratchpad Claude (/tmp/claude-<uid>/…, sans « .. »)
# dont base_url est http(s)://127.0.0.1, localhost ou [::1] (faux serveur jira-fake).
loopback_config() {
  local cfg="" n=0 a prev=""
  for a in "$@"; do
    case "$a" in
      --config=*) cfg=${a#--config=}; n=$((n + 1)) ;;
    esac
    [ "$prev" = "--config" ] && { cfg=$a; n=$((n + 1)); }
    prev=$a
  done
  [ "$n" -eq 1 ] || return 1
  case "$cfg" in
    /tmp/claude-[0-9]*/*) ;;
    *) return 1 ;;
  esac
  case "$cfg" in *..*) return 1 ;; esac
  [ -f "$cfg" ] && [ ! -L "$cfg" ] || return 1
  local urls
  urls=$(grep -E '^[[:space:]]*base_url[[:space:]]*=' "$cfg") || return 1
  [ "$(printf '%s\n' "$urls" | wc -l)" -eq 1 ] || return 1
  printf '%s' "$urls" | grep -qE '^[[:space:]]*base_url[[:space:]]*=[[:space:]]*"https?://(127\.0\.0\.1|localhost|\[::1\])(:[0-9]+)?(/[^"]*)?"[[:space:]]*$'
}

# Fail-closed : sans jq, on ne peut pas analyser la commande → refus.
command -v jq >/dev/null 2>&1 || deny "jq est absent, impossible de vérifier la commande."

input=$(cat)
payload=$(printf '%s' "$input" | jq -r '.tool_input | (.command // "") + " " + (.url // "")') \
  || deny "entrée du hook illisible."

# 1. Aucune URL vers une instance Atlassian Cloud.
if printf '%s' "$payload" | grep -qi 'atlassian\.net'; then
  deny "accès à une instance Jira réelle (*.atlassian.net)."
fi

# 2. Ne pas toucher au garde-fou hors-ligne (JIRA_RUST_OFFLINE, positionné par .cargo/config.toml).
if printf '%s' "$payload" | grep -q 'JIRA_RUST_OFFLINE'; then
  deny "modification du garde-fou JIRA_RUST_OFFLINE."
fi

# 3. Exécution directe du binaire jira-rust (hors cargo, donc non bridée) : --mock obligatoire
#    dans le même segment de commande. On découpe sur ; & | et retours à la ligne, on saute les
#    préfixes (VAR=val, env, exec, nohup, time, timeout N, sudo) et on regarde le premier mot.
while IFS= read -r segment; do
  read -r -a words <<<"$segment" || true
  i=0
  while [ "$i" -lt "${#words[@]}" ]; do
    w=${words[$i]}
    case "$w" in
      *=*|env|exec|nohup|time|sudo|command|'('|'{') i=$((i + 1)) ;;
      timeout) i=$((i + 2)) ;;
      *) break ;;
    esac
  done
  [ "$i" -lt "${#words[@]}" ] || continue
  prog=${words[$i]##*/}
  if [ "$prog" = "jira-rust" ]; then
    case " $segment " in
      *" --mock "*) ;;
      *) loopback_config "${words[@]:$((i + 1))}" \
           || deny "exécution directe du binaire jira-rust sans --mock (exception : --config <scratchpad> vers 127.0.0.1/localhost)." ;;
    esac
  fi
done < <(printf '%s\n' "$payload" | tr ';&|' '\n\n\n')

exit 0
