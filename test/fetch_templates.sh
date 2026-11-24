#!/usr/bin/env bash
# Populate test/templates/<name>/ with official venue templates. Idempotent.
set -u
T="$(cd "$(dirname "$0")" && pwd)/templates"
DOC=/usr/share/doc/texlive-doc/latex
mkdir -p "$T"
dl() { curl -sSfL --connect-timeout 20 --max-time 120 -o "$2" "$1"; }
# run "<name>" function; builds in tmp dir, moves into place only on success
mk() {
  local n=$1 d="$T/$1"
  [ -d "$d" ] && { echo "skip $n"; return; }
  local tmp; tmp=$(mktemp -d "$T/.tmp.$n.XXXX")
  if ( set -e; "get_$n" "$tmp" ) && [ -s "$tmp/MAIN" ]; then
    mv "$tmp" "$d"; echo "ok   $n"
  else rm -rf "$tmp"; echo "FAIL $n" >&2; fi
}
unz() { # url name -> extracts into fresh empty dir, echoes dir
  local z; z=$(mktemp -d); dl "$1" "$z/a.zip"; mkdir "$z/x"; unzip -q "$z/a.zip" -d "$z/x"; echo "$z/x"
}
gz() { zcat "$1" > "$2"; }
get_acm() {
  local s=$DOC/acmart/samples o=$1
  gz $s/sample-sigconf.tex.gz $o/sample-sigconf.tex
  gz $s/sample-base.bib.gz $o/sample-base.bib
  cp $s/sample-franklin.png $o/
  [ -f $s/sampleteaser.pdf ] && cp $s/sampleteaser.pdf $o/
  echo sample-sigconf.tex > $o/MAIN
}
get_ieee() {
  gz $DOC/ieeetran/bare_conf.tex.gz $1/bare_conf.tex
  echo bare_conf.tex > $1/MAIN
}
get_lncs() {
  local x; x=$(unz "https://cms-resources.apps.public.k8s.springernature.io/springer-cms/rest/v1/content/27851904/data/LaTeX2e%20Proceedings%20Template%20ZIP")
  cp -r "$x"/*/. $1/ 2>/dev/null || cp -r "$x"/. $1/
  [ -f $1/samplepaper.tex ] || { echo no samplepaper >&2; return 1; }
  echo samplepaper.tex > $1/MAIN
}
get_neurips() {
  local x y
  for y in 2025 2024; do
    x=$(unz https://media.neurips.cc/Conferences/NeurIPS$y/Styles.zip) && break
  done
  cp -r "$x"/*/. $1/ 2>/dev/null || cp -r "$x"/. $1/
  local m; m=$(ls $1 | grep -E '^neurips_20[0-9]+\.tex$' | head -1)
  echo "$m" > $1/MAIN
}
get_icml() {
  local x y
  for y in 2026 2025; do
    x=$(unz https://media.icml.cc/Conferences/ICML$y/Styles/icml$y.zip) && break
  done
  cp -r "$x"/*/. $1/ 2>/dev/null || cp -r "$x"/. $1/
  echo example_paper.tex > $1/MAIN
}
get_acl() {
  local x; x=$(unz https://github.com/acl-org/acl-style-files/archive/refs/heads/master.zip)
  cp -r "$x"/*/. $1/
  rm -rf $1/.git $1/.github
  echo acl_latex.tex > $1/MAIN
}
get_usenix() {
  local b=https://files.usenix.org/s3fs-public
  dl $b/usenix2019_v3.sty $1/usenix2019_v3.sty
  dl $b/usenix2019_v3.2.tex $1/usenix2019_v3.2.tex
  echo usenix2019_v3.2.tex > $1/MAIN
}
for n in acm ieee lncs neurips icml acl usenix; do mk $n; done
