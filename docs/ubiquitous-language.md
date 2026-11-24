# Ubiquitous language
(authored by agents unless marked 🧑)

- generation (gen): one numbered compile attempt; newer gens supersede older ones
    - cold gen: a fresh `lualatex` process
    - resumed gen: a clone of a snapshot continuing with new source
- snapshot: a forked, paused copy of TeX taken right after a page ships out
    - parked on FIFO `snap-PID` in the run dir; `go GEN` or `quit`
    - max-read: per source file, the highest line TeX had read when the snapshot was taken
    - stale: max-read reaches an edited line, so it cannot be resumed
- clone: the process a snapshot forks on `go`; the snapshot stays parked for reuse
- finisher: a fork of a running gen that closes its PDF right after the page just shipped
    - its PDF is a partial
- partial: PDF with the pages of a gen so far; done: PDF of a finished gen
- first-changed page: pages before it are identical to the previous gen's
- seed: aux files (`.aux`, `.bbl`, `.toc`, ...) a cold gen starts from
    - auto rerun: a cold gen started because a finished gen wrote aux files differing from its seed
- shadow: copies of unsaved editor buffers; TeX reads these instead of the files on disk
- run dir: per-project scratch dir under `/dev/shm`; holds `ev`, `snap-*`, `shadow/`, `gN`, `gNpP`
