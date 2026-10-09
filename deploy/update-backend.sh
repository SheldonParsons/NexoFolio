#!/usr/bin/env bash
# Standalone updater; shared pure-shell helpers are also used by upload-server.sh.
# Never source release manifests, deployment state, or server .env files.
set +x

die() { printf '%s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "Missing required tool: $1"; }
hash_stream() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum | awk '{print $1}';
  else shasum -a 256 | awk '{print $1}'; fi
}
hash_file() { hash_stream < "$1"; }
hex_random() { od -An -N "$1" -tx1 /dev/urandom | tr -d ' \n'; }
file_size() { wc -c < "$1" | tr -d '[:space:]'; }
kv() { awk -F '\t' -v wanted="$2" '$1 == wanted {print $2}' "$1"; }
valid_hash() { [[ $1 =~ ^[a-f0-9]{64}$ ]]; }
valid_release() { [[ $1 =~ ^[0-9]{8}T[0-9]{6}Z-[a-f0-9]{16}$ ]]; }
valid_platform() { [[ $1 == linux/amd64 || $1 == linux/arm64 ]]; }
private_file() {
  local mode
  [[ -f $1 && ! -L $1 ]] || die 'A regular environment file is required.'
  mode=$(stat -c '%a' "$1" 2>/dev/null) || mode=$(stat -f '%Lp' "$1")
  [[ $mode == 600 || $mode == 400 ]] || die 'Environment file permissions must be 0600 or 0400.'
}
quiet() { "$@" >/dev/null 2>&1; }

validate_kv() {
  # Exact keys, two fields per row, no duplicates, CRs, shell evaluation or empties.
  awk -F '\t' -v allowed="$2" '
    BEGIN {n=split(allowed,a," "); for(i=1;i<=n;i++) expected[a[i]]=1}
    NF!=2 || !($1 in expected) || seen[$1]++ || $2=="" || /\r/ {bad=1}
    END {if(NR!=n || bad) exit 1}
  ' "$1" || die 'Invalid or duplicate metadata fields.'
}
manifest_check() {
  local manifest=$1 key value
  validate_kv "$manifest" 'format release platform image image_id image_sha image_size compose_sha env_sha migrations_sha sources_sha readme_sha updater_sha source_sha git_head'
  [[ $(kv "$manifest" format) == 2 ]] || die 'Unsupported release manifest format.'
  valid_release "$(kv "$manifest" release)" || die 'Invalid release ID.'
  valid_platform "$(kv "$manifest" platform)" || die 'Unsupported release architecture.'
  value=$(kv "$manifest" image_id)
  [[ $value =~ ^sha256:[a-f0-9]{64}$ ]] || die 'Invalid image ID.'
  [[ $(kv "$manifest" image) == "nexofolio-backend:release-${value:7:16}" ]] || die 'Image tag/ID mismatch.'
  value=$(kv "$manifest" image_size)
  [[ $value =~ ^[1-9][0-9]{0,10}$ ]] && (( value <= 8589934592 )) || die 'Invalid image archive size.'
  value=$(kv "$manifest" git_head)
  [[ $value =~ ^[a-f0-9]{40,64}$ ]] || die 'Invalid Git provenance.'
  for key in image_sha compose_sha env_sha migrations_sha sources_sha readme_sha updater_sha source_sha; do
    valid_hash "$(kv "$manifest" "$key")" || die 'Invalid artifact checksum.'
  done
}
migrations_check() {
  awk -F '\t' '
    NF!=2 || length($1)!=64 || $1 !~ /^[a-f0-9]+$/ ||
      $2 !~ /^modules\/[a-z]+\/adapter\/migrations\/[A-Za-z0-9_-]+\.sql$/ || seen[$2]++ {bad=1}
    END {if(NR==0 || bad) exit 1}
  ' "$1" || die 'Invalid migration inventory.'
}
template_check() {
  awk -F '=' '
    BEGIN {expected["POSTGRES_PASSWORD"]="REPLACE_ON_SERVER";
      expected["DATABASE_URL"]="postgres://nexofolio:REPLACE_ON_SERVER@db:5432/nexofolio";
      expected["NEXOFOLIO_SESSION_KEY"]=""; expected["NEXOFOLIO_EMERGENCY_PASSWORD_HASH"]="";
      expected["NEXOFOLIO_CATALOG_MODEL_API_KEY"]=""}
    /^[A-Z_]+=/ {if($1 in expected) {value=substr($0,index($0,"=")+1); if(value!=expected[$1] || seen[$1]++) bad=1}}
    END {for(k in expected) if(seen[k]!=1) bad=1; if(bad) exit 1}
  ' "$1" || die 'The example configuration contains non-placeholder secret settings.'
}

clean_compose_environment() {
  local name
  while IFS= read -r name; do
    case "$name" in NEXOFOLIO_*|COMPOSE_*|DATABASE_URL|POSTGRES_PASSWORD|POSTGRES_IMAGE|BACKEND_IMAGE) unset "$name" ;; esac
  done < <(compgen -v)
}
load_environment() {
  local line key value seen='|'
  private_file "$1"
  clean_compose_environment
  while IFS= read -r line || [[ -n $line ]]; do
    case "$line" in ''|\#*) continue ;; esac
    [[ $line != *$'\r'* && $line =~ ^([A-Z][A-Z0-9_]*)=(.*)$ ]] || die 'Unsupported .env syntax; use one KEY=value per line.'
    key=${BASH_REMATCH[1]}; value=${BASH_REMATCH[2]}
    case "$key" in COMPOSE_PROJECT_NAME|POSTGRES_PASSWORD|POSTGRES_IMAGE|DATABASE_URL|BACKEND_IMAGE|NEXOFOLIO_*) ;; *) die 'Unexpected environment key.' ;; esac
    [[ $seen != *"|$key|"* ]] || die 'Duplicate environment key.'
    seen="$seen$key|"
    if [[ $value == \'* ]]; then
      [[ ${#value} -ge 2 && $value == *\' ]] || die 'Unclosed single quote in .env.'
      value=${value:1:${#value}-2}
    elif [[ $value == \"* ]]; then
      die 'Use bare values or single quotes in .env; double-quoted expansion is not supported.'
    fi
    export "$key=$value"
  done < "$1"
  [[ ${COMPOSE_PROJECT_NAME:-} =~ ^[a-z0-9][a-z0-9_-]*$ ]] || die 'Configure the dedicated Compose project.'
  [[ ${NEXOFOLIO_PG_VOLUME:-} =~ ^[A-Za-z0-9][A-Za-z0-9_.-]*$ ]] || die 'Configure the dedicated database volume.'
  [[ ${POSTGRES_IMAGE:-} =~ ^[A-Za-z0-9][A-Za-z0-9_./:@-]*$ ]] || die 'Configure the local PostgreSQL image.'
  [[ ${POSTGRES_PASSWORD:-} =~ ^[A-Za-z0-9_-]{16,128}$ && $POSTGRES_PASSWORD != *REPLACE* ]] || die 'Keep the generated database password or use 16-128 URL-safe characters.'
  [[ ${DATABASE_URL:-} == "postgres://nexofolio:$POSTGRES_PASSWORD@db:5432/nexofolio" ]] || die 'DATABASE_URL must use the dedicated database and matching password.'
  [[ ${NEXOFOLIO_SESSION_KEY:-} =~ ^[a-fA-F0-9]{64}$ && -n ${NEXOFOLIO_ZENTAO_BASE_URL:-} ]] || die 'Configure Zentao and the generated session key before deployment.'
  [[ ${NEXOFOLIO_HTTP_PORT:-} =~ ^[1-9][0-9]{0,4}$ ]] && (( NEXOFOLIO_HTTP_PORT <= 65535 )) || die 'Invalid backend host port.'
}

download() {
  local code
  code=$(curl --disable --fail --silent --show-error --noproxy '*' --proto '=https' --max-redirs 0 \
    --connect-timeout 15 --max-time 1800 --retry 2 --max-filesize "$3" \
    --output "$2" --write-out '%{http_code}' "$OSS_URL/$1" 2>/dev/null) || die 'OSS download failed; raw output withheld.'
  [[ $code == 200 && $(file_size "$2") -le $3 ]] || die 'Invalid OSS response; redirects are not followed.'
}
fetch_blob() {
  local digest=$1 extension=$2 limit=$3 destination="$DEPLOY_ROOT/cache/$1.$2"
  if [[ -f $destination && $(hash_file "$destination") == "$digest" ]]; then
    [[ $(file_size "$destination") -le $limit ]] || die 'Cached object is too large.'
  else
    download "blobs/$digest.$extension" "$WORK/blob.partial" "$limit"
    [[ $(hash_file "$WORK/blob.partial") == "$digest" ]] || die 'Artifact SHA-256 mismatch.'
    mv "$WORK/blob.partial" "$destination"
  fi
  BLOB_PATH=$destination
}
read_release() {
  local requested=$1
  if [[ $requested != - ]]; then
    valid_release "$requested" || die 'Invalid requested version.'
    download "releases/$requested/manifest.tsv" "$WORK/manifest.tsv" 32768
  else
    download "latest-${SERVER_PLATFORM/\//-}.tsv" "$WORK/manifest.tsv" 32768
  fi
  manifest_check "$WORK/manifest.tsv"
  RELEASE=$(kv "$WORK/manifest.tsv" release)
  [[ $requested == - || $requested == "$RELEASE" ]] || die 'Requested version does not match manifest.'
  [[ $(kv "$WORK/manifest.tsv" platform) == "$SERVER_PLATFORM" ]] || die 'Release architecture does not match the server.'
  RELEASE_DIR="$DEPLOY_ROOT/releases/$RELEASE"
  mkdir -p "$RELEASE_DIR"
  if [[ -f $RELEASE_DIR/manifest.tsv ]]; then
    cmp -s "$WORK/manifest.tsv" "$RELEASE_DIR/manifest.tsv" || die 'An immutable release changed.'
  else mv "$WORK/manifest.tsv" "$RELEASE_DIR/manifest.tsv"; fi
}
release_files() {
  local key filename limit
  manifest_check "$RELEASE_DIR/manifest.tsv"
  [[ $(kv "$RELEASE_DIR/manifest.tsv" platform) == "$SERVER_PLATFORM" ]] || die 'Release architecture mismatch.'
  for key in compose env migrations; do
    case "$key" in compose) filename=compose.yaml; limit=131072 ;; env) filename=.env.example; limit=32768 ;; migrations) filename=migrations.tsv; limit=131072 ;; esac
    fetch_blob "$(kv "$RELEASE_DIR/manifest.tsv" "${key}_sha")" txt "$limit"
    cp "$BLOB_PATH" "$WORK/$filename"
    mv "$WORK/$filename" "$RELEASE_DIR/$filename"
  done
  migrations_check "$RELEASE_DIR/migrations.tsv"
}

state_read() {
  local key value
  S_OWNER=- S_PROJECT=- S_VOLUME=- S_PG_IMAGE=- S_PG_ID=- S_DB_SHA=- S_CURRENT=- S_PREVIOUS=-
  S_ENV_SHA=- S_ENV_FILE=- S_PENDING=- S_PENDING_ENV_SHA=- S_PENDING_ENV_FILE=- S_BACKUP=- S_BACKUP_SHA=- S_PHASE=idle
  [[ -f $DEPLOY_ROOT/state.tsv ]] || return 0
  validate_kv "$DEPLOY_ROOT/state.tsv" 'format owner project volume pg_image pg_id db_sha current previous env_sha env_file pending pending_env_sha pending_env_file backup backup_sha phase'
  while IFS=$'\t' read -r key value; do
    case "$key" in
      format) [[ $value == 2 ]] || die 'Unsupported deployment state.' ;;
      owner) [[ $value =~ ^[a-f0-9]{32}$ ]] || die 'Invalid owner state.'; S_OWNER=$value ;;
      project) [[ $value =~ ^[a-z0-9][a-z0-9_-]*$ ]] || die 'Invalid project state.'; S_PROJECT=$value ;;
      volume) [[ $value =~ ^[A-Za-z0-9][A-Za-z0-9_.-]*$ ]] || die 'Invalid volume state.'; S_VOLUME=$value ;;
      pg_image) [[ $value =~ ^[A-Za-z0-9][A-Za-z0-9_./:@-]*$ ]] || die 'Invalid database image state.'; S_PG_IMAGE=$value ;;
      pg_id) [[ $value =~ ^sha256:[a-f0-9]{64}$ ]] || die 'Invalid database ID state.'; S_PG_ID=$value ;;
      db_sha) valid_hash "$value" || die 'Invalid database fingerprint.'; S_DB_SHA=$value ;;
      current|previous|pending) [[ $value == - ]] || valid_release "$value" || die 'Invalid saved release.'
        case "$key" in current) S_CURRENT=$value ;; previous) S_PREVIOUS=$value ;; pending) S_PENDING=$value ;; esac ;;
      env_sha|pending_env_sha|backup_sha) [[ $value == - ]] || valid_hash "$value" || die 'Invalid saved checksum.'
        case "$key" in env_sha) S_ENV_SHA=$value ;; pending_env_sha) S_PENDING_ENV_SHA=$value ;; backup_sha) S_BACKUP_SHA=$value ;; esac ;;
      env_file|pending_env_file|backup) [[ $value == - || $value =~ ^(configs|backups)/[0-9]{8}T[0-9]{6}Z-[a-f0-9]{16}\.(env|dump)$ ]] || die 'Invalid saved local path.'
        case "$key" in env_file) S_ENV_FILE=$value ;; pending_env_file) S_PENDING_ENV_FILE=$value ;; backup) S_BACKUP=$value ;; esac ;;
      phase) case "$value" in idle|preparing|migrating|starting-api) S_PHASE=$value ;; *) die 'Invalid deployment phase.' ;; esac ;;
    esac
  done < "$DEPLOY_ROOT/state.tsv"
}
state_write() {
  {
    printf 'format\t2\nowner\t%s\nproject\t%s\nvolume\t%s\npg_image\t%s\npg_id\t%s\ndb_sha\t%s\n' "$S_OWNER" "$S_PROJECT" "$S_VOLUME" "$S_PG_IMAGE" "$S_PG_ID" "$S_DB_SHA"
    printf 'current\t%s\nprevious\t%s\nenv_sha\t%s\nenv_file\t%s\n' "$S_CURRENT" "$S_PREVIOUS" "$S_ENV_SHA" "$S_ENV_FILE"
    printf 'pending\t%s\npending_env_sha\t%s\npending_env_file\t%s\nbackup\t%s\nbackup_sha\t%s\nphase\t%s\n' "$S_PENDING" "$S_PENDING_ENV_SHA" "$S_PENDING_ENV_FILE" "$S_BACKUP" "$S_BACKUP_SHA" "$S_PHASE"
  } > "$WORK/state.tsv"
  mv "$WORK/state.tsv" "$DEPLOY_ROOT/state.tsv"
}
dc() { docker compose --env-file /dev/null -p "$COMPOSE_PROJECT_NAME" -f "$RELEASE_DIR/compose.yaml" "$@"; }
run_admin_command() {
  local action=$1 output status key
  # Compose v2.27 has no `run --pull` flag. The verified release's migrate
  # service inherits pull_policy: never and has no build section.
  if output=$(dc run --rm --no-deps -T migrate nexofolio-admin "$action" 2>&1); then
    return 0
  else status=$?; fi
  printf 'Backend command failed: action=%s exit=%s\n' "$action" "$status" >&2
  # Emit fixed categories and known setting NAMES only, never raw command output.
  case "$output" in
    *'unknown flag:'*) printf 'Reason: COMPOSE_OPTION_UNSUPPORTED\n' >&2 ;;
    *'ZenTao requires HTTPS'*) printf 'Reason: ZENTAO_URL_POLICY; use HTTPS without embedded credentials or query parameters.\n' >&2 ;;
    *'InvalidInput'*) printf 'Reason: CONFIGURATION_REJECTED\n' >&2 ;;
    *'error while loading shared libraries'*) printf 'Reason: RUNTIME_LIBRARY_MISSING\n' >&2 ;;
    *'Operation not permitted'*) printf 'Reason: RUNTIME_PERMISSION_DENIED\n' >&2 ;;
    *'panicked at'*) printf 'Reason: BACKEND_PANIC\n' >&2 ;;
    *'OCI runtime'*|*'exec format error'*) printf 'Reason: CONTAINER_RUNTIME_START_FAILURE\n' >&2 ;;
    *'Unavailable'*) printf 'Reason: BACKEND_DEPENDENCY_UNAVAILABLE\n' >&2 ;;
    *) printf 'Reason: UNCLASSIFIED_FAILURE; raw output withheld to protect credentials.\n' >&2 ;;
  esac
  for key in DATABASE_URL NEXOFOLIO_BIND_ADDR NEXOFOLIO_LOG NEXOFOLIO_ZENTAO_BASE_URL \
    NEXOFOLIO_SESSION_KEY NEXOFOLIO_EMERGENCY_PASSWORD_HASH NEXOFOLIO_DB_MAX_CONNECTIONS \
    NEXOFOLIO_DB_TIMEOUT_MS NEXOFOLIO_REQUEST_TIMEOUT_MS NEXOFOLIO_SHUTDOWN_TIMEOUT_MS \
    NEXOFOLIO_MCP_ALLOWED_HOSTS NEXOFOLIO_MCP_ALLOWED_ORIGINS NEXOFOLIO_MODEL_MAX_CONCURRENCY \
    NEXOFOLIO_FIELD_CONCURRENCY NEXOFOLIO_DESCRIPTION_CONCURRENCY NEXOFOLIO_CURATION_IDLE_SECONDS \
    NEXOFOLIO_MODEL_THINKING NEXOFOLIO_MODEL_JSON_OUTPUT; do
    [[ $output != *"$key"* ]] || printf 'Setting mentioned: %s\n' "$key" >&2
  done
  return "$status"
}
select_image() {
  export BACKEND_IMAGE NEXOFOLIO_PLATFORM
  BACKEND_IMAGE=$(kv "$RELEASE_DIR/manifest.tsv" image)
  NEXOFOLIO_PLATFORM=$(kv "$RELEASE_DIR/manifest.tsv" platform)
  EXPECTED_IMAGE_ID=$(kv "$RELEASE_DIR/manifest.tsv" image_id)
}
archive_config_id() {
  # docker save has a compatibility manifest even when the local store uses an
  # OCI index ID. Classic graphdriver engines identify the image by config hash.
  # Restrict this reader to our single-tag exports; never extract paths to disk.
  local archive=$1 tag=$2 config_path config_digest actual_digest
  config_path=$(tar -xzOf "$archive" manifest.json 2>/dev/null | awk -v tag="$tag" '
    {text=text $0}
    END {
      rest=text; count=0
      while(match(rest,/"Config"[[:space:]]*:[[:space:]]*"[^"\\]*"/)) {
        value=substr(rest,RSTART,RLENGTH); sub(/^"Config"[[:space:]]*:[[:space:]]*"/,"",value); sub(/"$/,"",value)
        config=value; count++; rest=substr(rest,RSTART+RLENGTH)
      }
      if(count!=1) exit 1
      rest=text; count=0
      while(match(rest,/"RepoTags"[[:space:]]*:[[:space:]]*\[[^]]*\]/)) {
        value=substr(rest,RSTART,RLENGTH); sub(/^"RepoTags"[[:space:]]*:[[:space:]]*/,"",value); gsub(/[[:space:]]/,"",value)
        if(value!="[\"" tag "\"]") exit 1
        count++; rest=substr(rest,RSTART+RLENGTH)
      }
      if(count!=1) exit 1
      print config
    }
  ') || return 1
  if [[ $config_path =~ ^blobs/sha256/([a-f0-9]{64})$ ]]; then config_digest=${BASH_REMATCH[1]}
  elif [[ $config_path =~ ^([a-f0-9]{64})\.json$ ]]; then config_digest=${BASH_REMATCH[1]}
  else return 1; fi
  actual_digest=$(tar -xzOf "$archive" "$config_path" 2>/dev/null | hash_stream) || return 1
  [[ $actual_digest == "$config_digest" ]] || return 1
  printf 'sha256:%s\n' "$config_digest"
}
image_matches_release() {
  local info image_platform archive digest size config_id
  VERIFIED_IMAGE_ID=''
  info=$(docker image inspect --format '{{.Id}} {{.Os}}/{{.Architecture}}' "$BACKEND_IMAGE" 2>/dev/null) || return 1
  read -r VERIFIED_IMAGE_ID image_platform <<< "$info"
  [[ $VERIFIED_IMAGE_ID =~ ^sha256:[a-f0-9]{64}$ && $image_platform == "$NEXOFOLIO_PLATFORM" ]] || return 1
  [[ $VERIFIED_IMAGE_ID != "$EXPECTED_IMAGE_ID" ]] || return 0
  digest=$(kv "$RELEASE_DIR/manifest.tsv" image_sha)
  size=$(kv "$RELEASE_DIR/manifest.tsv" image_size)
  archive="$DEPLOY_ROOT/cache/$digest.tar.gz"
  [[ -f $archive && $(file_size "$archive") == "$size" && $(hash_file "$archive") == "$digest" ]] || return 1
  config_id=$(archive_config_id "$archive" "$BACKEND_IMAGE") || return 1
  [[ $VERIFIED_IMAGE_ID == "$config_id" ]]
}
verify_health() {
  local container actual route body expected
  image_matches_release || return 1
  container=$(dc ps --quiet api 2>/dev/null) || return 1
  [[ $container =~ ^[a-f0-9]{12,64}$ ]] || return 1
  actual=$(docker inspect --format '{{.Image}}' "$container" 2>/dev/null) || return 1
  [[ $actual == "$VERIFIED_IMAGE_ID" ]] || return 1
  for route in live ready; do
    expected=$route; [[ $route != live ]] || expected=alive
    body=$(dc exec -T api curl --fail --silent --max-time 5 "http://127.0.0.1:8080/health/$route" 2>/dev/null) || return 1
    [[ $body == "{\"status\":\"$expected\"}" ]] || return 1
  done
}

deploy_release() {
  local pg_info pg_id pg_platform db_sha env_sha containers volume_owner image_info image_id image_platform stamp old container mounts backup_path
  release_files
  # Snapshot .env once. Validation, fingerprinting and Compose all use these bytes.
  private_file "$DEPLOY_ROOT/.env"
  cp "$DEPLOY_ROOT/.env" "$WORK/runtime.env"
  chmod 600 "$WORK/runtime.env"
  load_environment "$WORK/runtime.env"
  select_image
  STAGE='Compose and local database checks'
  quiet dc --profile tools config --quiet
  pg_info=$(docker image inspect --format '{{.Id}} {{.Os}}/{{.Architecture}}' "$POSTGRES_IMAGE" 2>/dev/null) || die 'The required PostgreSQL image is not available locally.'
  read -r pg_id pg_platform <<< "$pg_info"
  [[ $pg_id =~ ^sha256:[a-f0-9]{64}$ && $pg_platform == "$SERVER_PLATFORM" ]] || die 'PostgreSQL image architecture mismatch.'
  db_sha=$(printf '%s\n%s\n' "$POSTGRES_PASSWORD" "$DATABASE_URL" | hash_stream)
  env_sha=$(hash_file "$WORK/runtime.env")
  if [[ $S_OWNER != - ]]; then
    [[ $S_PROJECT == "$COMPOSE_PROJECT_NAME" && $S_VOLUME == "$NEXOFOLIO_PG_VOLUME" && $S_PG_IMAGE == "$POSTGRES_IMAGE" && $S_PG_ID == "$pg_id" && $S_DB_SHA == "$db_sha" ]] || die 'Database/project identity changed; automatic replacement is refused.'
    [[ $S_PENDING == - || ( $ACTION != rollback && $S_PENDING == "$RELEASE" ) ]] || die 'Retry the recorded incomplete release before choosing another version.'
  else
    containers=$(docker ps -aq --filter "label=com.docker.compose.project=$COMPOSE_PROJECT_NAME" 2>/dev/null)
    [[ -z $containers ]] || die 'Compose project is already in use.'
    if quiet docker volume inspect "$NEXOFOLIO_PG_VOLUME"; then die 'Database volume already exists; no existing data will be adopted.'; fi
  fi
  if [[ $S_CURRENT != - ]]; then
    old="$DEPLOY_ROOT/releases/$S_CURRENT/migrations.tsv"
    [[ -f $old && $(hash_file "$old") == "$(kv "$DEPLOY_ROOT/releases/$S_CURRENT/manifest.tsv" migrations_sha)" ]] || die 'Saved migration inventory is missing or corrupt.'
    if [[ $ACTION == rollback ]]; then
      cmp -s "$old" "$RELEASE_DIR/migrations.tsv" || die 'Schema migrations differ; automatic rollback is refused.'
    else
      awk -F '\t' 'NR==FNR {old[$2]=$1; next} {if($2 in old) {if(old[$2]!=$1) bad=1; delete old[$2]}} END {for(k in old) bad=1; if(bad) exit 1}' "$old" "$RELEASE_DIR/migrations.tsv" || die 'A previous migration was changed or removed.'
    fi
  fi
  if [[ $S_PENDING == - && $S_CURRENT == "$RELEASE" && $S_ENV_SHA == "$env_sha" ]]; then
    verify_health || die 'The recorded release is not healthy; inspect its service before updating.'
    printf 'Already deployed and healthy: %s\n' "$RELEASE"
    return
  fi
  STAGE='Offline backend image import'
  printf 'Checking the selected local backend image...\n'
  if ! image_matches_release; then
    fetch_blob "$(kv "$RELEASE_DIR/manifest.tsv" image_sha)" tar.gz "$(kv "$RELEASE_DIR/manifest.tsv" image_size)"
    [[ $(file_size "$BLOB_PATH") == "$(kv "$RELEASE_DIR/manifest.tsv" image_size)" ]] || die 'Image archive size mismatch.'
    quiet docker load --input "$BLOB_PATH"
  fi
  if ! image_matches_release; then
    # Only non-secret image metadata is reported; configuration/env stay hidden.
    image_info=$(docker image inspect --format '{{.Id}} {{.Os}}/{{.Architecture}}' "$BACKEND_IMAGE" 2>/dev/null) || image_info=unavailable
    printf 'Expected release identity: %s %s\nImported image identity: %s\n' "$EXPECTED_IMAGE_ID" "$SERVER_PLATFORM" "$image_info" >&2
    die 'Imported image does not match the verified archive and target platform.'
  fi
  if [[ $S_OWNER == - ]]; then
    S_OWNER=$(hex_random 16); S_PROJECT=$COMPOSE_PROJECT_NAME; S_VOLUME=$NEXOFOLIO_PG_VOLUME
    S_PG_IMAGE=$POSTGRES_IMAGE; S_PG_ID=$pg_id; S_DB_SHA=$db_sha
  fi
  if [[ $S_PENDING == - ]]; then
    S_PENDING=$RELEASE; S_PHASE=preparing; S_BACKUP=-; S_BACKUP_SHA=-
  fi
  if [[ $S_PENDING_ENV_SHA != "$env_sha" || $S_PENDING_ENV_FILE == - ]]; then
    stamp="$(date -u +%Y%m%dT%H%M%SZ)-$(hex_random 8)"
    S_PENDING_ENV_FILE="configs/$stamp.env"; S_PENDING_ENV_SHA=$env_sha
    cp "$WORK/runtime.env" "$DEPLOY_ROOT/$S_PENDING_ENV_FILE"
  fi
  state_write
  if volume_owner=$(docker volume inspect --format '{{index .Labels "io.nexofolio.owner"}}' "$S_VOLUME" 2>/dev/null); then
    [[ $volume_owner == "$S_OWNER" ]] || die 'Database volume ownership mismatch.'
  else
    [[ $S_CURRENT == - && $S_PHASE == preparing && $S_BACKUP == - ]] || die 'The recorded data volume is missing; refusing to replace it.'
    quiet docker volume create --label "io.nexofolio.owner=$S_OWNER" "$S_VOLUME"
  fi
  STAGE='Dedicated PostgreSQL startup'
  printf 'Starting PostgreSQL from its existing local image...\n'
  quiet dc up -d --no-build --pull never --no-recreate --wait --wait-timeout 60 db
  container=$(dc ps --quiet db 2>/dev/null)
  [[ $container =~ ^[a-f0-9]{12,64}$ ]] || die 'Expected exactly one database container.'
  [[ $(docker inspect --format '{{.Image}}' "$container" 2>/dev/null) == "$S_PG_ID" ]] || die 'Database container image mismatch.'
  mounts=$(docker inspect --format '{{range .Mounts}}{{if eq .Destination "/var/lib/postgresql/data"}}{{.Name}}{{end}}{{end}}' "$container" 2>/dev/null)
  [[ $mounts == "$S_VOLUME" ]] || die 'Database container volume mismatch.'
  STAGE='Backend configuration check'
  run_admin_command check-config
  STAGE='API stop and database backup'
  printf 'Stopping API writes and backing up PostgreSQL...\n'
  quiet dc stop --timeout 30 api
  if [[ $S_BACKUP == - ]]; then
    backup_path="backups/$(basename "$S_PENDING_ENV_FILE" .env).dump"
    dc exec -T db pg_dump -U nexofolio -d nexofolio -Fc --no-owner --no-acl > "$WORK/database.dump" 2>/dev/null
    [[ -s $WORK/database.dump ]] || die 'Database backup is empty.'
    dc exec -T db pg_restore --list < "$WORK/database.dump" >/dev/null 2>&1
    mv "$WORK/database.dump" "$DEPLOY_ROOT/$backup_path"
    S_BACKUP=$backup_path; S_BACKUP_SHA=$(hash_file "$DEPLOY_ROOT/$S_BACKUP")
    state_write
  fi
  [[ -f $DEPLOY_ROOT/$S_BACKUP && $(hash_file "$DEPLOY_ROOT/$S_BACKUP") == "$S_BACKUP_SHA" ]] || die 'Original pre-migration backup is missing or corrupt.'
  if [[ $ACTION != rollback ]]; then
    S_PHASE=migrating; state_write; STAGE='Database migration'
    printf 'Applying database migrations...\n'
    run_admin_command migrate
  fi
  S_PHASE=starting-api; state_write; STAGE='API startup and health verification'
  printf 'Starting API and verifying its image and health...\n'
  if ! quiet dc up -d --no-deps --no-build --pull never --wait --wait-timeout 60 api || ! verify_health; then
    quiet dc stop --timeout 30 api || true
    die 'Candidate API failed verification; stop was attempted and recovery state was retained.'
  fi
  [[ $S_CURRENT == "$RELEASE" ]] || S_PREVIOUS=$S_CURRENT
  S_CURRENT=$RELEASE; S_ENV_SHA=$S_PENDING_ENV_SHA; S_ENV_FILE=$S_PENDING_ENV_FILE
  S_PENDING=-; S_PENDING_ENV_SHA=-; S_PENDING_ENV_FILE=-; S_PHASE=idle
  state_write
  printf 'Deployed and healthy: %s\nAPI upstream: http://127.0.0.1:%s\n' "$RELEASE" "$NEXOFOLIO_HTTP_PORT"
}

updater_main() {
  set -euo pipefail
  export LC_ALL=C
  umask 077
  ACTION=update; REQUESTED=-; DEPLOY_ROOT=${DEPLOY_DIR:-/home/nexofolio/backend}
  OSS_URL=${OSS_BASE_URL:-https://asynctest.oss-cn-shenzhen.aliyuncs.com/linux_files/nexofolio/backend}
  while (( $# )); do
    case "$1" in
      init|update|status|rollback) ACTION=$1; shift ;;
      --deploy-dir) [[ $# -ge 2 ]] || die 'Missing deployment directory.'; DEPLOY_ROOT=$2; shift 2 ;;
      --oss-base-url) [[ $# -ge 2 ]] || die 'Missing OSS URL.'; OSS_URL=$2; shift 2 ;;
      --release) [[ $# -ge 2 ]] || die 'Missing release ID.'; REQUESTED=$2; shift 2 ;;
      -h|--help) printf 'Usage: bash update-backend.sh [init|update|status|rollback] [--release ID] [--deploy-dir DIR] [--oss-base-url HTTPS_URL]\n'; return ;;
      *) die 'Unknown argument; use --help.' ;;
    esac
  done
  [[ $REQUESTED == - ]] || valid_release "$REQUESTED" || die 'Invalid requested release.'
  OSS_URL=${OSS_URL%/}
  [[ $OSS_URL =~ ^https://[A-Za-z0-9.-]+(:443)?(/[A-Za-z0-9/_-]+)?$ && $OSS_URL != *..* ]] || die 'OSS URL must be HTTPS without credentials, queries or fragments.'
  for command in docker curl tar awk sort cmp od tr wc stat; do need "$command"; done
  if ! command -v sha256sum >/dev/null 2>&1; then need shasum; fi
  mkdir -p "$DEPLOY_ROOT"
  DEPLOY_ROOT=$(cd "$DEPLOY_ROOT" && pwd -P)
  [[ ! -f $DEPLOY_ROOT/state.json ]] || die 'A legacy deployment state exists; do not mix updater formats.'
  LOCK_DIR="$DEPLOY_ROOT/.deploy-lock"
  mkdir "$LOCK_DIR" 2>/dev/null || die 'A deployment lock exists; check for an active updater before removing a stale lock.'
  WORK=''; STAGE=initialization
  trap 'code=$?; trap - EXIT; [[ -z $WORK ]] || rm -rf -- "$WORK"; rmdir "$LOCK_DIR" 2>/dev/null || true; if (( code )); then printf "%s failed; raw output withheld. State and backups were retained; no database restore was attempted.\n" "$STAGE" >&2; fi; exit "$code"' EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM
  WORK=$(mktemp -d "$DEPLOY_ROOT/.work.XXXXXX")
  mkdir -p "$DEPLOY_ROOT/cache" "$DEPLOY_ROOT/releases" "$DEPLOY_ROOT/configs" "$DEPLOY_ROOT/backups"
  local version major minor line password session
  version=$(docker compose version --short 2>/dev/null)
  [[ $version =~ ^v?([0-9]+)\.([0-9]+) ]] || die 'Cannot determine Compose version.'
  major=${BASH_REMATCH[1]}; minor=${BASH_REMATCH[2]}
  (( major > 2 || (major == 2 && minor >= 27) )) || die 'Docker Compose v2.27+ is required.'
  SERVER_PLATFORM=$(docker info --format '{{.OSType}}/{{.Architecture}}' 2>/dev/null)
  case "$SERVER_PLATFORM" in linux/x86_64) SERVER_PLATFORM=linux/amd64 ;; linux/aarch64) SERVER_PLATFORM=linux/arm64 ;; esac
  valid_platform "$SERVER_PLATFORM" || die 'A supported Linux Docker daemon is required.'
  state_read
  if [[ $ACTION == status ]]; then
    printf 'Last successful release: %s\n' "$S_CURRENT"
    if [[ $S_PENDING != - ]]; then printf 'Incomplete deployment: %s (%s)\n' "$S_PENDING" "$S_PHASE"
    elif [[ $S_CURRENT != - ]]; then
      RELEASE_DIR="$DEPLOY_ROOT/releases/$S_CURRENT"
      manifest_check "$RELEASE_DIR/manifest.tsv"
      load_environment "$DEPLOY_ROOT/$S_ENV_FILE"; select_image
      verify_health || die 'Current API health/image verification failed.'
      printf 'Current API health and image verified.\n'
    fi
    return
  fi
  if [[ $ACTION == rollback ]]; then
    [[ $REQUESTED == - && $S_PREVIOUS != - && $S_PENDING == - ]] || die 'Rollback needs a completed previous version and no pending deployment.'
    RELEASE=$S_PREVIOUS; RELEASE_DIR="$DEPLOY_ROOT/releases/$RELEASE"
  else read_release "$REQUESTED"; fi
  if [[ $ACTION == init ]]; then
    [[ ! -e $DEPLOY_ROOT/.env && ! -L $DEPLOY_ROOT/.env ]] || die 'Environment file exists; it was not overwritten.'
    release_files; template_check "$RELEASE_DIR/.env.example"
    password=$(hex_random 24); session=$(hex_random 32)
    while IFS= read -r line || [[ -n $line ]]; do
      case "$line" in
        BACKEND_IMAGE=*|NEXOFOLIO_PLATFORM=*) continue ;;
        POSTGRES_PASSWORD=*) line="POSTGRES_PASSWORD=$password" ;;
        DATABASE_URL=*) line="DATABASE_URL=postgres://nexofolio:$password@db:5432/nexofolio" ;;
        NEXOFOLIO_SESSION_KEY=*) line="NEXOFOLIO_SESSION_KEY=$session" ;;
      esac
      printf '%s\n' "$line"
    done < "$RELEASE_DIR/.env.example" > "$WORK/initial.env"
    mv "$WORK/initial.env" "$DEPLOY_ROOT/.env"
    unset password session line
    printf 'Created protected .env. Fill in Zentao and model settings locally, then run update. No containers were started.\n'
  else deploy_release; fi
}

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then updater_main "$@"; fi
