# Spec Delta

## Purpose

Makes every external benchmark dataset the Lab scores against traceable to a machine-pinned public
source with known license terms and contamination notes, fetched outside git only on the owner's
request, and verified byte for byte before any result is computed from it.

## ADDED Requirements

### Requirement: The owner's manifest stays local and is created from a committed template
The working manifest `lab/datasets.manifest.toml` SHALL be ignored by git and SHALL never be committed, so license clearances and recorded checksums stay on the owner's machine. The repository SHALL hold `lab/datasets.manifest.example.toml` with the same entries, every `license.status = "pending"` and no owner-recorded checksums. `monoagens-lab datasets init` SHALL create the local manifest from the template and SHALL refuse to overwrite an existing one.

#### Scenario: A fresh clone has no local manifest
- **WHEN** a Lab command that needs the manifest runs and `lab/datasets.manifest.toml` is absent
- **THEN** it exits non-zero telling the owner to run `monoagens-lab datasets init`, and no default test depends on the local manifest

#### Scenario: Initializing over an existing manifest
- **WHEN** the owner runs `monoagens-lab datasets init` and `lab/datasets.manifest.toml` already exists
- **THEN** nothing is overwritten and the command exits non-zero

#### Scenario: The local manifest is staged by mistake
- **WHEN** the owner runs `git status` after clearing licenses and recording checksums
- **THEN** `lab/datasets.manifest.toml` is not listed, while the template stays tracked with every license `pending`

### Requirement: A manifest describes every external dataset
The local manifest and its template SHALL record every external dataset in `standard_benchmark_registry()`: id, domain, pinned source(s), availability, contamination risk, license SPDX and terms, license status, file SHA-256 and sizes, and converter/probe and scorer versions (`2026-11-17-run-3.md`). Unverified commit SHAs and sizes SHALL stay `pending_pin` until recorded by owner-run `datasets pin` or `--record-checksums`, never hand-typed by agents.

#### Scenario: The registry and the manifest disagree
- **WHEN** the benchmark registry names an external dataset the manifest lacks, or the manifest names one the registry lacks
- **THEN** every Lab command that reads the manifest fails, naming each mismatched id

#### Scenario: A dataset spans two primary sources
- **WHEN** a benchmark splits conversations and evaluation queries across separate repositories (such as `group_mem_bench` across Hugging Face and GitHub)
- **THEN** the manifest records each pinned source and revision, and `monoagens-lab datasets list` reports both sources cleanly

#### Scenario: A manifest entry awaits owner pinning, license clearance, or checksums
- **WHEN** a dataset has `pending_pin` files, `pending` license status, or missing owner checksums
- **THEN** `monoagens-lab datasets list` reports its license status, contamination risk, and pinning/checksum state

### Requirement: Machine pinning records upstream revisions and sizes without manual guessing
`monoagens-lab datasets pin [ids]` SHALL query upstream GitHub or Hugging Face APIs directly on explicit owner request and write exact commit SHAs, file paths, byte sizes, and available upstream LFS hashes into `lab/datasets.manifest.toml` without downloading corpus files or clearing licenses.

#### Scenario: Pinning a repository resolves commit SHA and sizes automatically
- **WHEN** the owner runs `monoagens-lab datasets pin locomo`
- **THEN** the command writes the live resolved commit SHA and file byte sizes into `lab/datasets.manifest.toml` while leaving `license.status` unchanged

### Requirement: Dataset files never enter git
Dataset files SHALL live only under `lab/datasets/`, which SHALL be ignored by git. The Lab
SHALL NOT read external dataset files from any other path, and SHALL NOT read
`lab/scenarios/sealed/` while handling datasets.

#### Scenario: Dataset files are fetched
- **WHEN** the owner fetches a dataset
- **THEN** its files land under `lab/datasets/<dataset path>/` and `git status` lists none of them

### Requirement: Only cleared licenses are fetched
`monoagens-lab datasets fetch` SHALL download a dataset only when its manifest license status is
`cleared`. A dataset with status `pending` or `refused` SHALL NOT be downloaded, even when
named explicitly.

#### Scenario: Fetching a dataset whose license is pending
- **WHEN** the owner runs `monoagens-lab datasets fetch locomo` and its license status is `pending`
- **THEN** nothing is downloaded and the command exits non-zero, naming the dataset and its license status

### Requirement: Dataset network access happens only on explicit owner commands
Network access for external datasets SHALL happen only inside `monoagens-lab datasets pin` and
`monoagens-lab datasets fetch`. No other Lab command and no default test SHALL open a network
connection for datasets.

#### Scenario: A benchmark runs while a dataset is missing
- **WHEN** `monoagens-lab benchmark --suite standard` runs and a required available dataset is absent
- **THEN** the command exits non-zero naming the dataset and the fetch command to run, and downloads nothing

### Requirement: Fetched files are verified before use
`monoagens-lab datasets fetch` and `monoagens-lab datasets verify` SHALL check every manifest
file's SHA-256 and size. A dataset SHALL count as verified only when every file matches, no
manifest file is missing, and its converted form, if cached, was made by the manifest's
converter version.

#### Scenario: A file was altered after download
- **WHEN** one byte of a fetched dataset file changes
- **THEN** `monoagens-lab datasets verify` reports that dataset unverified, naming the file and both hashes, and exits non-zero

#### Scenario: A fetch is interrupted
- **WHEN** a download stops part way
- **THEN** no partial file is left under the dataset's final path, and the dataset stays unverified

### Requirement: Checksums missing from the source are recorded once by the owner
When the source publishes no checksums, `monoagens-lab datasets fetch --record-checksums` SHALL
compute them on a fetch of a cleared dataset and write them into the manifest, together with the
fetch date. It SHALL refuse when the manifest already holds checksums for that dataset. Later
fetches SHALL verify against the recorded values.

#### Scenario: Recording checksums twice
- **WHEN** the owner runs `--record-checksums` for a dataset whose manifest entry already has checksums
- **THEN** the command changes nothing and exits non-zero, telling the owner to use `datasets verify`

### Requirement: The manifest names datasets that are not yet published
A dataset whose public release cannot be confirmed (`pis`, `proact`, `mpbench`) SHALL be marked
`unavailable` in the manifest. It SHALL stay in the benchmark report as not run, with that
reason, and SHALL NOT be replaced silently by another dataset.

#### Scenario: A registered benchmark has no public release
- **WHEN** the manifest marks `pis`, `proact`, or `mpbench` as `unavailable`
- **THEN** the standard suite report lists each as not run with reason `unavailable`, and its domain scorecard shows the gap
