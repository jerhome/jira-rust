#!/usr/bin/env bash
# Build release Linux statique (x86_64-unknown-linux-musl) de jira-rust.
#
# Usage : scripts/build-linux.sh [--allow-gcc-fallback] [--check-only]
#
# Produit :
#   dist/jira-rust-<version>-x86_64-linux-musl
#   dist/jira-rust-<version>-x86_64-linux-musl.sha256
#
# Prérequis : rustup, cible musl (ajoutée automatiquement), compilateur C musl
# (`musl-gcc`, paquet Debian/Ubuntu `musl-tools`) pour compiler aws-lc-sys
# (fournisseur crypto de rustls).
#
# --allow-gcc-fallback : si musl-gcc est absent, compile le code C avec le gcc
# de l'hôte (en-têtes glibc). Fonctionne en pratique sur Debian 12, mais n'est
# pas la configuration recommandée pour une distribution.
#
# --check-only : ne compile rien ; rejoue les contrôles sur le binaire déjà
# présent dans dist/ (version, --help, --mock doctor, statique, pas de /home).
#
# Le binaire n'est vérifié qu'avec --version, --help et --mock doctor :
# aucune connexion à une instance Jira.
set -euo pipefail

TARGET=x86_64-unknown-linux-musl
BIN=jira-rust
ALLOW_GCC_FALLBACK=0
CHECK_ONLY=0
USED_FALLBACK=0

for arg in "$@"; do
    case "$arg" in
        --allow-gcc-fallback) ALLOW_GCC_FALLBACK=1 ;;
        --check-only) CHECK_ONLY=1 ;;
        -h|--help) sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "Argument inconnu : $arg" >&2; exit 2 ;;
    esac
done

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

die() { echo "ERREUR : $*" >&2; exit 1; }
step() { printf '\n==> %s\n' "$*"; }

VERSION="$(cargo metadata --format-version 1 --no-deps --offline \
    | sed -n 's/.*"name":"jira-tui","version":"\([^"]*\)".*/\1/p')"
[[ -n "$VERSION" ]] || die "version de jira-tui introuvable"

# --- Vérifications ----------------------------------------------------------
verify() {
    local bin="$1" version="$2" out
    step "Vérifications de $bin"

    local file_out
    file_out="$(file -b "$bin")"
    echo "file : $file_out"
    grep -Eq 'static(-pie)? linked|statically linked' <<<"$file_out" \
        || die "le binaire n'est pas statique (file)"
    if command -v ldd >/dev/null; then
        local ldd_out
        ldd_out="$(ldd "$bin" 2>&1 || true)"
        echo "ldd  : $ldd_out"
        grep -Eq 'statically linked|not a dynamic executable' <<<"$ldd_out" \
            || die "ldd signale des dépendances dynamiques"
    fi

    # Uniquement des commandes sans réseau.
    out="$("$bin" --version)" || die "--version a échoué"
    echo "version : $out"
    grep -Fq "$version" <<<"$out" \
        || die "--version ('$out') ne correspond pas à Cargo.toml ($version)"

    out="$("$bin" --help)" || die "--help a échoué"
    for flag in --read-only --mock; do
        grep -Fq -- "$flag" <<<"$out" || die "--help ne liste pas $flag"
    done
    echo "--help : --read-only et --mock présents"

    "$bin" --mock doctor || die "--mock doctor a échoué"

    if grep -aq '/home' "$bin"; then
        echo "Occurrences de /home :" >&2
        grep -aoE '[[:print:]]{0,40}/home[[:print:]]{0,60}' "$bin" | head -5 >&2
        die "le binaire contient une chaîne de chemin /home"
    fi
    echo "aucun chemin /home dans le binaire"
    echo "Contrôles OK"
}

# --- Distribution -----------------------------------------------------------
NAME="$BIN-$VERSION-x86_64-linux-musl"
if [[ "$CHECK_ONLY" == 1 ]]; then
    [[ -f "dist/$NAME" ]] || die "dist/$NAME absent (lancez d'abord un build)"
    verify "dist/$NAME" "$VERSION"
    [[ -s "dist/$NAME.sha256" ]] || die "dist/$NAME.sha256 absent"
    (cd dist && sha256sum -c "$NAME.sha256") || die "SHA-256 invalide"
    step "Terminé (--check-only)"
    exit 0
fi

# --- Outils -----------------------------------------------------------------
step "Vérification des outils"
for tool in cargo rustup file sha256sum; do
    command -v "$tool" >/dev/null || die "outil manquant : $tool"
done

if ! rustup target list --installed | grep -qx "$TARGET"; then
    echo "Ajout de la cible $TARGET"
    rustup target add "$TARGET"
fi

# Compilateur C pour la cible musl (aws-lc-sys).
CC_VAR="CC_${TARGET//-/_}"
if [[ -n "${!CC_VAR:-}" ]]; then
    echo "Compilateur C musl imposé : $CC_VAR=${!CC_VAR}"
elif command -v x86_64-linux-musl-gcc >/dev/null; then
    echo "Compilateur C musl : x86_64-linux-musl-gcc"
elif command -v musl-gcc >/dev/null; then
    export "$CC_VAR=musl-gcc"
    echo "Compilateur C musl : musl-gcc"
elif [[ "$ALLOW_GCC_FALLBACK" == 1 ]] && command -v gcc >/dev/null; then
    export "$CC_VAR=gcc"
    USED_FALLBACK=1
    echo "!!! AVERTISSEMENT : musl-gcc absent, REPLI sur gcc (en-têtes glibc)." >&2
    echo "!!! Build non recommandé pour une distribution publiée." >&2
else
    cat >&2 <<'EOF'
ERREUR : aucun compilateur C pour la cible musl (musl-gcc).
Il est nécessaire pour compiler aws-lc-sys (crypto TLS de rustls).

Installez-le (Debian/Ubuntu) :
    sudo apt-get install musl-tools

ou relancez avec --allow-gcc-fallback (non recommandé pour une distribution).
EOF
    exit 1
fi


# Reproductibilité : chemins neutralisés dans le binaire, date fixe.
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct 2>/dev/null || echo 0)}"
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$ROOT=. --remap-path-prefix=$HOME/.cargo=~/.cargo"

step "Build release $BIN $VERSION ($TARGET)"
START=$(date +%s)
cargo build --release --locked --target "$TARGET" -p jira-tui --bin "$BIN"
END=$(date +%s)
OUT="target/$TARGET/release/$BIN"
[[ -x "$OUT" ]] || die "binaire absent : $OUT"

verify "$OUT" "$VERSION"

step "Copie dans dist/"
mkdir -p dist
install -m 0755 "$OUT" "dist/$NAME"
(cd dist && sha256sum "$NAME" > "$NAME.sha256" && sha256sum -c "$NAME.sha256")

SIZE=$(stat -c %s "dist/$NAME")
step "Terminé"
[[ "$USED_FALLBACK" == 1 ]] && echo "!!! Build réalisé avec le REPLI gcc (musl-gcc absent)." >&2
echo "Binaire  : dist/$NAME"
echo "Taille   : $SIZE octets ($(numfmt --to=iec "$SIZE"))"
echo "SHA-256  : $(cut -d' ' -f1 "dist/$NAME.sha256")"
echo "Durée    : $((END - START)) s"
