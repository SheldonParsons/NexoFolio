#!/usr/bin/env bash
# Test-only command boundary. Never calls a real Docker daemon or OSS endpoint.
set -euo pipefail
name=${0##*/}
mkdir -p "$FAKE_ROOT"
event() { printf '%s\n' "$*" >> "$FAKE_ROOT/events"; }
fail_stage() { if [[ ${FAKE_FAIL:-} == "$1" ]]; then printf 'fixture-sensitive-error\n' >&2; exit 1; fi; }
key() { local value=$1; value=${value//\//_}; value=${value//:/_}; printf '%s' "$value"; }
image_get() {
  if [[ $1 == postgres:17.4 ]]; then printf 'sha256:%064d' 0
  else cat "$FAKE_ROOT/image-$(key "$1")"; fi
}
case "$name" in
  python|python3|jq) touch "$FAKE_ROOT/forbidden-interpreter"; exit 99 ;;
  git)
    if [[ " $* " == *' rev-parse '* ]]; then printf '%040d\n' 0
    else exec "$REAL_GIT" "$@"; fi
    exit ;;
  cp)
    "$REAL_CP" "$@"
    if [[ -n ${FAKE_SOURCE_RACE:-} && ${1:-} == "$FAKE_SOURCE_RACE" ]]; then printf '\nconcurrent edit\n' >> "$FAKE_SOURCE_RACE"; fi
    exit ;;
  ossutil)
    action=$1; shift
    if [[ $action == stat ]]; then
      fail_stage oss-stat
      relative=${1#oss://asynctest/linux_files/nexofolio/backend/}
      if [[ -f $FAKE_OSS/$relative ]]; then exit 0; fi
      printf 'NoSuchKey\n' >&2; exit 1
    fi
    [[ $action == cp ]] || exit 98
    source_file=$1; relative=${2#oss://asynctest/linux_files/nexofolio/backend/}
    [[ $relative != "$2" && $relative != *..* ]] || exit 98
    if [[ $relative == */manifest.tsv ]]; then fail_stage upload-manifest; fi
    event "oss-copy $relative"
    mkdir -p "$FAKE_OSS/$(dirname "$relative")"
    "$REAL_CP" "$source_file" "$FAKE_OSS/$relative"
    exit ;;
  curl)
    output='' url=''
    while (( $# )); do
      case "$1" in
        --output) output=$2; shift 2 ;;
        https://*) url=$1; shift ;;
        *) shift ;;
      esac
    done
    relative=${url#https://asynctest.oss-cn-shenzhen.aliyuncs.com/linux_files/nexofolio/backend/}
    [[ $relative != "$url" && $relative != *..* && -f $FAKE_OSS/$relative ]] || exit 22
    event "oss-get $relative"
    "$REAL_CP" "$FAKE_OSS/$relative" "$output"
    printf '200'
    exit ;;
  docker) ;;
  *) exit 98 ;;
esac

case "${1:-}" in
  build)
    event build
    tag='' digest='' context=${!#}
    while (( $# )); do
      case "$1" in --tag) tag=$2; shift 2 ;; --label) digest=${2#io.nexofolio.source-sha256=}; shift 2 ;; *) shift ;; esac
    done
    [[ ! -e $context/apps/.env && ! -e $context/.env ]] || exit 97
    printf 'sha256:%s' "$digest" > "$FAKE_ROOT/image-$(key "$tag")"
    exit ;;
  tag) image_get "$2" > "$FAKE_ROOT/image-$(key "$3")"; exit ;;
  save)
    printf 'image\t%s\nid\t%s\n' "$2" "$(image_get "$2")"
    exit ;;
  load)
    event load
    gzip -dc "$3" > "$FAKE_ROOT/loaded.tsv"
    tag=$(awk -F '\t' '$1=="image"{print $2}' "$FAKE_ROOT/loaded.tsv")
    id=$(awk -F '\t' '$1=="id"{print $2}' "$FAKE_ROOT/loaded.tsv")
    printf '%s' "$id" > "$FAKE_ROOT/image-$(key "$tag")"
    exit ;;
  image)
    [[ $2 == inspect ]] || exit 98
    id=$(image_get "${!#}" 2>/dev/null) || exit 1
    printf '%s linux/amd64\n' "$id"
    exit ;;
  info) printf 'linux/x86_64\n'; exit ;;
  ps) [[ ! -f $FAKE_ROOT/project ]] || printf '%064d\n' 1; exit ;;
  volume)
    if [[ $2 == inspect ]]; then [[ -f $FAKE_ROOT/volume-owner ]] || exit 1; cat "$FAKE_ROOT/volume-owner"; exit; fi
    [[ $2 == create ]] || exit 98
    event volume-create
    printf '%s' "${4#io.nexofolio.owner=}" > "$FAKE_ROOT/volume-owner"
    printf '%s' "${!#}" > "$FAKE_ROOT/volume-name"
    exit ;;
  inspect)
    if [[ $3 == '{{.Image}}' ]]; then
      if [[ ${!#} == "$(printf '%064d' 1)" ]]; then cat "$FAKE_ROOT/api-image"
      else image_get postgres:17.4; fi
    else cat "$FAKE_ROOT/volume-name"; fi
    exit ;;
  compose)
    shift
    if [[ ${1:-} == version ]]; then printf '2.27.0\n'; exit; fi
    while (( $# )); do
      case "$1" in --env-file|-p|-f|--profile) shift 2 ;; *) break ;; esac
    done
    action=$1; shift
    case "$action" in
      config) exit 0 ;;
      up)
        [[ " $* " == *' --pull never '* && " $* " == *' --no-build '* ]] || exit 96
        if [[ ${!#} == api ]]; then
          event api-up; touch "$FAKE_ROOT/api-running"; image_get "$BACKEND_IMAGE" > "$FAKE_ROOT/api-image"; fail_stage api-up
        else event db-up; touch "$FAKE_ROOT/project"; fi
        exit ;;
      stop) event stop; rm -f "$FAKE_ROOT/api-running"; exit ;;
      ps)
        if [[ ${!#} == api ]]; then printf '%064d\n' 1; else printf '%064d\n' 2; fi
        exit ;;
      run)
        # Match the actual v2.27 CLI instead of accepting newer-only flags.
        if [[ " $* " == *' --pull '* ]]; then printf 'unknown flag: --pull\n' >&2; exit 16; fi
        if [[ ${!#} == check-config ]]; then event check-config; fail_stage check-config
        else event migrate; fail_stage migrate; fi
        exit ;;
      exec)
        case " $* " in
          *' pg_dump '*) event backup; fail_stage backup; printf 'fixture-custom-dump\n' ;;
          *' pg_restore '*) event backup-check; fail_stage backup-check; input=$(cat); [[ $input == fixture-custom-dump ]] || exit 1 ;;
          *' curl '*) event health; fail_stage health; if [[ ${!#} == */live ]]; then printf '{"status":"alive"}'; else printf '{"status":"ready"}'; fi ;;
          *) exit 98 ;;
        esac
        exit ;;
    esac
esac
exit 98
