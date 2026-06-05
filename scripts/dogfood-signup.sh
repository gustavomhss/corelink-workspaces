#!/usr/bin/env bash
# dogfood-signup.sh — become CoreLink customer #1 and wire clw to prod.
#
# Mirrors corelink-server/scripts/e2e-clerk-signup.sh (the PROVEN prod chain:
# Clerk user -> signup-worker webhook -> tenant + PAT in D1 -> pat_plaintext in
# Clerk publicMetadata) — but creates a REAL, PERMANENT user (no cleanup) and
# writes the credentials straight into ~/.clw/config.toml (chmod 600).
#
# Usage:
#   bash scripts/dogfood-signup.sh <your-email>
#
# Reads CLERK_SECRET_KEY from ../corelink-server/.env.local.
# Hard rules: the PAT and the Clerk key are NEVER echoed (only last4/length).
set -euo pipefail

EMAIL="${1:?usage: dogfood-signup.sh <email>}"
ENV_FILE="${HOME}/Documents/HuGR/corelink-server/.env.local"
CLERK_API="https://api.clerk.com/v1"
CLW_DIR="${HOME}/.clw"
ENDPOINT="https://corelink-api.humangr.com"

[[ -f "${ENV_FILE}" ]] || { echo "ERRO: ${ENV_FILE} nao existe"; exit 1; }
CLERK_SECRET_KEY="$(grep '^CLERK_SECRET_KEY=' "${ENV_FILE}" | head -1 | cut -d= -f2-)"
[[ -n "${CLERK_SECRET_KEY}" ]] || { echo "ERRO: CLERK_SECRET_KEY ausente do .env.local"; exit 1; }
for cmd in curl jq openssl; do command -v "${cmd}" >/dev/null || { echo "ERRO: ${cmd} ausente"; exit 1; }; done

echo "[1/4] procurando usuario Clerk existente para ${EMAIL}"
USER_ID="$(curl -sS --max-time 15 -H "Authorization: Bearer ${CLERK_SECRET_KEY}" \
  "${CLERK_API}/users?email_address=${EMAIL}" | jq -r '.[0].id // empty')"

if [[ -z "${USER_ID}" ]]; then
  echo "      nao existe -- criando usuario REAL (permanente) no Clerk"
  PASSWORD="Clw!$(openssl rand -hex 12)"
  CREATE_RESP="$(curl -sS --max-time 30 -X POST \
    -H "Authorization: Bearer ${CLERK_SECRET_KEY}" -H "Content-Type: application/json" \
    -d "{\"email_address\":[\"${EMAIL}\"],\"password\":\"${PASSWORD}\"}" \
    "${CLERK_API}/users")"
  USER_ID="$(printf '%s' "${CREATE_RESP}" | jq -r '.id // empty')"
  [[ -n "${USER_ID}" ]] || { echo "ERRO ao criar usuario: $(printf '%s' "${CREATE_RESP}" | head -c 300)"; exit 1; }
  mkdir -p "${CLW_DIR}" && chmod 700 "${CLW_DIR}"
  ( umask 177 && printf 'email = "%s"\npassword = "%s"\n' "${EMAIL}" "${PASSWORD}" > "${CLW_DIR}/clerk-login.txt" )
  echo "      user criado: ${USER_ID} (senha salva em ~/.clw/clerk-login.txt, modo 600)"
else
  echo "      user ja existe: ${USER_ID}"
fi

echo "[2/4] aguardando signup-worker provisionar tenant + PAT (ate 90s)"
DEADLINE=$(( $(date +%s) + 90 ))
TENANT_ID=""; PAT=""; REGION=""
while [[ $(date +%s) -lt ${DEADLINE} ]]; do
  META="$(curl -sS --max-time 15 -H "Authorization: Bearer ${CLERK_SECRET_KEY}" "${CLERK_API}/users/${USER_ID}")"
  TENANT_ID="$(printf '%s' "${META}" | jq -r '.public_metadata.tenant_id // empty')"
  PAT="$(printf '%s' "${META}" | jq -r '.public_metadata.pat_plaintext // empty')"
  REGION="$(printf '%s' "${META}" | jq -r '.public_metadata.region // empty')"
  [[ -n "${TENANT_ID}" && -n "${PAT}" ]] && break
  sleep 3
done
[[ -n "${TENANT_ID}" && -n "${PAT}" ]] || { echo "ERRO: tenant/PAT nao provisionados em 90s -- conferir wrangler tail do signup-worker"; exit 1; }
PAT_LAST4="${PAT: -4}"
echo "      tenant: ${TENANT_ID} (regiao: ${REGION:-?}) / PAT: ${#PAT} chars (...${PAT_LAST4})"

echo "[3/4] gravando ~/.clw/config.toml (600)"
mkdir -p "${CLW_DIR}/cache" && chmod 700 "${CLW_DIR}"
( umask 177 && cat > "${CLW_DIR}/config.toml" <<EOF
endpoint = "${ENDPOINT}"
tenant = "${TENANT_ID}"
token = "${PAT}"
cache_dir = "${HOME}/.clw/cache"
EOF
)
( umask 177 && cat > "${CLW_DIR}/dogfood.env" <<EOF
export CLW_ENDPOINT="${ENDPOINT}"
export CLW_TENANT="${TENANT_ID}"
export CLW_TOKEN="${PAT}"
EOF
)
echo "      config.toml + dogfood.env gravados"

echo "[4/4] smoke: GET autenticado no endpoint"
CODE="$(curl -sS -o /dev/null -w "%{http_code}" --max-time 15 \
  -H "Authorization: Bearer ${PAT}" "${ENDPOINT}/v1/cas/${TENANT_ID}/0000000000000000000000000000000000000000000000000000000000000000")"
if [[ "${CODE}" == "404" ]]; then
  echo "      OK: 404 (auth aceito, blob inexistente -- exatamente o esperado). PRONTO."
elif [[ "${CODE}" == "401" ]]; then
  echo "      FALHA: 401 -- PAT nao aceito pelo data plane."; exit 1
else
  echo "      ATENCAO: HTTP ${CODE} -- inesperado, investigar antes do dogfood."; exit 1
fi
echo ""
echo "Proximos passos:"
echo "  source ~/.clw/dogfood.env"
echo "  cd ~/Documents/HuGR/corelink-workspaces"
echo "  cargo test -p clw-conformance --features live"
echo "  ./target/release/clw snapshot <projeto> --name <nome>"
