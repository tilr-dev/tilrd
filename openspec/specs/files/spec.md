# files Specification

## Purpose

The files protocol (feature `"files"`): a client looks at and changes the files of the server's machine for a file browser and an editor with `FsList`, `FsStat`, `FsRead`, `FsWrite`, `FsMkdir`, `FsRename`, `FsDelete`, `FsWatch` and `FsUnwatch`, confined to the shared directories. The two-lane client queue that carries `FsData` is specified in the `protocol` spec; `tilr edit` / `HostEvent` live in the `clipboard-host-events` spec.

## Requirements

### Requirement: The feature and its limit are announced
The server SHALL list `"files"` in `HelloAck.features` and set `HelloAck.fs_max_bytes` to the most one `FsRead` returns or one `FsWrite` takes: `[files] max_bytes` (a size like `"600K"` in powers of 1024, or an integer), 64 MiB when unset. An unparsable `max_bytes` MUST be a configuration error.

**Verified by**: `files::a_client_lists_a_directory_and_looks_at_its_entries` and the unit test `config::tests::files_are_on_for_the_home_directory_unless_said_otherwise` (crates/tilrd/src/config.rs).

#### Scenario: Default server
- **WHEN** a client says `Hello` to a server started without file options
- **THEN** `HelloAck.features` contains `"files"` and `fs_max_bytes` is 67108864

#### Scenario: Configured limit
- **WHEN** the configuration file has `[files]` `max_bytes = "1MB"`
- **THEN** the limit is 1048576; `max_bytes = "lots"` is refused as a configuration error

### Requirement: Files can be turned off
With `--no-files` or `[files] enabled = false` the feature MUST be absent from `HelloAck.features` and `fs_max_bytes` MUST be 0; every `Fs*` request MUST then go unanswered and change nothing, and the connection MUST stay open.

**Verified by**: `files::files_can_be_turned_off`.

#### Scenario: Requests while off
- **WHEN** the server runs with `--no-files` (or `[files] enabled = false`) and a client sends `FsList`, an `FsWrite` for an existing file, then `Ping`
- **THEN** the next answer is `Pong`, no `FsEntries`, `FsAck` or `Error` comes, and the file is unchanged

### Requirement: Each request is answered by its request id
Every `Fs*` request SHALL be answered with its own answer type (`FsEntries`, `FsInfo`, `FsData`, `FsAck`) carrying the client's `request_id`, and a failure MUST be told in that answer's `error`, never with the general `Error` message. The error SHALL read `<path as the request named it>: <reason>`.

**Verified by**: `files::a_client_lists_a_directory_and_looks_at_its_entries` and `files::directories_are_made_entries_renamed_and_deleted`.

#### Scenario: Listing a missing directory
- **WHEN** a client sends `FsList` for `~/missing` with `request_id` 6
- **THEN** it gets `FsEntries` with `request_id` 6 and `error` `"~/missing: no such file or directory"`

#### Scenario: Reasons
- **WHEN** a request fails on the file system
- **THEN** the reason is one of `no such file or directory`, `permission denied`, `already exists`, `directory not empty`, `not a directory`, `is a directory`, or the OS error's own text

### Requirement: Paths are absolute or start at the home directory
A request path SHALL be absolute, or `~` / `~/...` for the home directory of the user the server runs as; anything else (relative, `~other`, empty) MUST be refused with `not an absolute path`. Answers SHALL name the directory or entry absolute, `~` expanded, `..` applied and the directories on the way without links.

**Verified by**: `files::a_client_lists_a_directory_and_looks_at_its_entries` and the unit test `files::tests::paths_are_absolute_or_from_home` (crates/tilrd/src/files.rs).

#### Scenario: Relative path
- **WHEN** a client sends `FsStat` for `docs`
- **THEN** the `FsInfo.error` contains `not an absolute path`

#### Scenario: Through `..` and a link
- **WHEN** a client lists `<home>/docs/sub/..`, or `~/to-sub` where `to-sub` links to `docs/sub`
- **THEN** `FsEntries.path` is `<home>/docs`, respectively `<home>/docs/sub`, with that directory's entries

### Requirement: Only the shared directories are served
The server SHALL serve only the directories in `[files] roots` (default `["~"]`; a root missing at startup is left out with a warning). A path leading outside every root, by `..` or a link, MUST be refused for every request with `<path>: outside the directories this server shares`, whether or not it exists.

**Verified by**: `files::nothing_outside_the_shared_directories_is_served` and UC-071 (`uc_071.py::symbolic_links_are_listed_followed_and_written_through_but_not_out_of_the_shared_directories`).

#### Scenario: A link that leads out
- **WHEN** roots are `["~/shared"]` and `~/shared/door` links to `../secret`, `~/shared/key-link` to `../secret/key`
- **THEN** `FsList ~/shared/door`, `FsRead ~/shared/key-link`, an `FsWrite` to `~/shared/key-link` and `FsWatch ~/shared/door` are refused as outside, and `~/secret/key` is unchanged

#### Scenario: Existence outside is not told
- **WHEN** roots are `["~/shared"]` and a client lists `~/missing` or stats `~/secret/key`
- **THEN** both errors are `<path>: outside the directories this server shares`, not "no such file"

### Requirement: Roots can be looked at but not changed
A root itself SHALL be listable and answer `FsStat`, but MUST be refused as outside when written, deleted or renamed; a symbolic link inside a root that points outside MAY itself be deleted, leaving its target.

**Verified by**: `files::nothing_outside_the_shared_directories_is_served` and `files::a_client_lists_a_directory_and_looks_at_its_entries`.

#### Scenario: A root cannot be removed
- **WHEN** roots are `["~/shared"]` and a client sends `FsDelete ~/shared` with `recursive` or `FsRename ~/shared ~/shared2`
- **THEN** both are refused as outside and the directory stays

#### Scenario: The link itself can go
- **WHEN** the client deletes `~/shared/door` (a link to `../secret`)
- **THEN** the `FsAck.error` is empty and `~/secret/key` still exists

### Requirement: Listings describe entries without following links
`FsList` SHALL answer `FsEntries`, entries sorted by name, each `FsEntry` with `name`, `kind` (`FILE_KIND_FILE`/`DIR`/`LINK`, `OTHER` for sockets, pipes, devices), `size` (a link's own), `mtime_ms`, `mode` (`& 0o7777`) and `target`: for a link what it points to (`OTHER` if broken), else its own kind. Listing a file MUST fail (`not a directory`).

**Verified by**: `files::a_client_lists_a_directory_and_looks_at_its_entries`, UC-071 (`uc_071.py::permission_bits_are_kept_by_writes_and_told_by_listings`).

#### Scenario: Links in a listing
- **WHEN** `~/docs` holds `a-link` -> `b.txt`, `b.txt` (12 bytes, mode 0640), `broken` -> `nowhere` and `sub/`
- **THEN** the entries are `a-link` (LINK, target FILE), `b.txt` (FILE, size 12, mode 0o640), `broken` (LINK, target OTHER), `sub` (DIR) in that order

### Requirement: Stat looks at the entry itself
`FsStat` SHALL answer `FsInfo` describing the named entry itself, a symbolic link not followed, with `path` absolute as in `FsEntries`; a root MAY be looked at too.

**Verified by**: `files::a_client_lists_a_directory_and_looks_at_its_entries`.

#### Scenario: Stat of a link
- **WHEN** a client sends `FsStat ~/docs/a-link`
- **THEN** `FsInfo.entry` is named `a-link` with kind LINK and target FILE

#### Scenario: Stat of the home root
- **WHEN** a client sends `FsStat ~`
- **THEN** `FsInfo.path` is the home directory and the entry's kind is DIR

### Requirement: Requests name files by bytes when given
Every path in a request SHALL have a bytes twin (`path_bytes`, `from_bytes`, `to_bytes`); when it is not empty the server MUST go by it and ignore the string, so a client that sends back the bytes it was given can read, stat, write, rename and delete a file whose name is not UTF-8.

**Verified by** (Linux only): `files::a_name_that_is_not_utf8_is_listed_with_its_bytes_and_named_by_them` and UC-071 (`uc_071.py::a_name_that_is_not_utf8_is_listed_so_that_it_can_be_read_back`).

#### Scenario: Read, stat, rename and delete by bytes
- **WHEN** `~/odd` holds a file named `bin\xff\xfe-\xc3(name.dat` and a client names it by `path_bytes`
- **THEN** `FsRead` returns its content, `FsStat` answers with the same `path_bytes`, `FsRename` by `from_bytes`/`to_bytes` and `FsDelete` by `path_bytes` succeed

### Requirement: Answers carry the bytes of names that are not UTF-8
In answers, a name or path that is not UTF-8 SHALL come with its raw bytes (`FsEntry.name_bytes`, `FsEntries.path_bytes`, `FsInfo.path_bytes`, `FsChanged.path_bytes`) and a string with U+FFFD for each invalid byte, for showing only; for a UTF-8 name the bytes field MUST be empty.

**Verified by** (Linux only): `files::a_name_that_is_not_utf8_is_listed_with_its_bytes_and_named_by_them` and UC-071 (`uc_071.py::odd_and_unicode_names_go_both_ways_and_keep_their_exact_bytes`).

#### Scenario: Listing an odd name
- **WHEN** `~/odd` holds `bin\xff\xfe-\xc3(name.dat` and `plain.txt` and a client lists it
- **THEN** the first is shown as `bin\u{fffd}\u{fffd}-\u{fffd}(name.dat` with `name_bytes` the raw name, and `plain.txt` has empty `name_bytes`

### Requirement: Reads come in chunks
`FsRead` SHALL answer with `FsData` chunks of at most 256 KiB, in order, the last with `last`, each carrying `total_size` (bytes the whole answer carries) and `file_size`. `offset` is where to start and `length` 0 means to the end; a part running past the end ends there, and a read starting past the end MUST answer one empty chunk with `last`.

**Verified by**: `files::a_file_is_read_in_chunks_whole_or_in_part` and UC-070 (`uc_070.py::files_of_zero_one_and_exactly_the_limit_go_both_ways_intact`).

#### Scenario: Whole file in three chunks
- **WHEN** a client reads a file of 2 x 256 KiB + 12345 bytes with offset 0 and length 0
- **THEN** it gets 3 `FsData` chunks whose data is the whole file

#### Scenario: Parts and the end
- **WHEN** a client reads offset 1000 length 5000, then 100 bytes from 10 before the end, then from 5 past the end
- **THEN** it gets bytes 1000..6000, the last 10 bytes, and one empty chunk with `last`

### Requirement: A read is bounded by the limit and fails whole
A read of more than `fs_max_bytes` MUST be refused (`too large to read at once: <size>, the server sends at most <limit>`), while a part within it MAY be read. Reading a directory MUST fail (`is a directory`), as MUST a file that shrinks while read. A failed read ends with an `FsData` with `last` and `error`; what came before is void.

**Verified by**: `files::a_file_is_read_in_chunks_whole_or_in_part`, UC-070 (`uc_070.py::a_file_one_byte_over_the_limit_is_refused_whole_and_read_in_parts`).

#### Scenario: Over the limit
- **WHEN** `max_bytes` is `"600K"` and a client reads a 700 KB file whole
- **THEN** the error contains `too large to read at once: 700 KB` and `at most 600 KB`
- **AND** a read from offset 400 KiB to the end returns the rest of the file

#### Scenario: A directory
- **WHEN** a client reads `~`
- **THEN** the error is `~: is a directory`

### Requirement: File content goes by the bulk lane
`FsData` SHALL go by the client's bulk lane (see the `protocol` spec), each chunk waiting for room there, so a slow client slows the read instead of growing the server's memory, and frames and echoes are not held up behind file content.

**Verified by**: UC-072 (`uc_072.py::parallel_reads_leave_the_terminal_responsive`, `uc_072.py::parallel_writes_leave_the_terminal_responsive`) and UC-070 (`uc_070.py::a_gigabyte_goes_up_and_comes_back_in_chunks_with_its_hash`).

#### Scenario: Transfers while typing
- **WHEN** several large files are read and written in parallel while the user types in a pane
- **THEN** the echoes keep arriving promptly and the files arrive intact (`uc_072.py::parallel_transfers_both_ways_leave_the_terminal_responsive_and_the_files_intact`)

### Requirement: Writes are whole or not at all
`FsWrite` SHALL take chunks of one `transfer_id` (path, `total_size`, `create`, `truncate` in the first) and answer one `FsAck` after the last chunk or as soon as it is refused (the rest then ignored). The server MUST write a temporary file next to the target and rename it over once `last` came with exactly `total_size` bytes: no reader sees half a file.

**Verified by**: `files::a_file_is_written_whole_or_not_at_all`, UC-070 (`uc_070.py::files_of_zero_one_and_exactly_the_limit_go_both_ways_intact`).

#### Scenario: A new file in chunks
- **WHEN** a client writes 2 x 256 KiB + 777 bytes to the missing `~/notes.md` with `create`
- **THEN** one `FsAck` with an empty error comes and the file holds those bytes

#### Scenario: Wrong amount of data
- **WHEN** a write announces `total_size` 100 and sends 50 bytes, or 150, with `last`
- **THEN** the ack's error contains `the write ended after 50 of 100 bytes`, respectively `carries more data than it announced`, and the file is unchanged

### Requirement: A finished write is on disk before it is acknowledged
Before an `FsAck` with an empty error the server SHALL have synced the temporary file's data to disk before the rename and the target's directory after it; a failed sync SHALL be the write's error.

**Verified by**: UC-046 (`uc_046.py::a_finished_write_is_synced_before_it_is_acknowledged`).

#### Scenario: Synced before the answer
- **WHEN** a client writes `~/notes.md` and the server's system calls are traced
- **THEN** an `fsync` of the temporary file comes before the `rename` onto `~/notes.md`, an `fsync` of the home directory after it, and both before the `FsAck`

### Requirement: Write modes and refusals
`create` without `truncate` MUST refuse an existing file (`already exists`); `truncate` without `create` MUST refuse a missing one (`no such file`); a missing parent (`no such file or directory`), a directory (`is a directory`), a file that is not regular (`not a regular file`) and more than `fs_max_bytes` (`too large: <size>, the server takes at most <limit>`, on the first chunk) MUST be refused with the file unchanged.

**Verified by**: `files::a_file_is_written_whole_or_not_at_all`.

#### Scenario: Create only
- **WHEN** `~/notes.md` exists and a client writes it with `create` and without `truncate`
- **THEN** the error is `~/notes.md: already exists`

#### Scenario: Over the limit
- **WHEN** `max_bytes` is `"1MB"` and a write announces 1100 KiB
- **THEN** the error contains `too large: 1.1 MB`, and the next answer the client gets is for its next request

### Requirement: Writes keep permissions and go through links
A replaced file SHALL keep its permission bits; a new file gets mode 0666 less the umask. A write to a symbolic link SHALL replace the file it points to, the link staying a link, provided that file is inside a root (else outside).

**Verified by**: `files::a_file_is_written_whole_or_not_at_all` and UC-071 (`uc_071.py::permission_bits_are_kept_by_writes_and_told_by_listings`).

#### Scenario: Replace keeps the mode
- **WHEN** `~/notes.md` has mode 0600 and a client writes `saved` with `truncate`
- **THEN** the content is `saved` and the mode is still 0600

#### Scenario: Through a link
- **WHEN** `~/link.md` links to `notes.md` and a client writes `through` to `~/link.md` with `truncate`
- **THEN** `notes.md` holds `through` and `link.md` is still a symbolic link

### Requirement: An abandoned write leaves the file and no temporary file
A write whose client's connection ends before its last chunk SHALL leave the target as it was and MUST remove its temporary file (named `.<name>.tilr-write-<12 hex digits>`).

**Verified by**: `files::a_file_is_written_whole_or_not_at_all`, itests `UC-046` (`uc_046.py::a_write_cut_by_a_reset_leaves_the_file_as_it_was_and_no_temporary_file`, `uc_046.py::a_write_cut_by_a_silent_network_leaves_the_file_as_it_was_and_no_temporary_file`).

#### Scenario: Client leaves in the middle
- **WHEN** a client sends the first of two chunks of a write to `~/notes.md` and its connection ends
- **THEN** `~/notes.md` keeps its previous content and no `.tilr-write-` file remains

### Requirement: Temporary files of writes cut short are swept at start
A write in progress SHALL note its temporary file in `writes/` of the runtime directory (0700). A server that starts or reloads MUST remove the temporary files noted by a process that is gone or by itself before an exec, and nothing whose name lacks `.tilr-write-`.

**Verified by**: the unit test `files::tests::the_temporary_file_of_a_write_whose_process_is_gone_is_swept` (crates/tilrd/src/files.rs), UC-064 (`uc_064.py::a_file_write_broken_by_an_update_leaves_the_file_as_it_was_and_the_retry_works`).

#### Scenario: Update in the middle of a write
- **WHEN** an update reloads the server while a write is half done
- **THEN** after the reload the file is as it was, no temporary file remains, and a retried write succeeds

#### Scenario: A live writer is left alone
- **WHEN** a note in `writes/` names a process that is alive, or names a file without `.tilr-write-`
- **THEN** the sweep leaves that temporary file, respectively that file, in place

### Requirement: Directories are made and entries renamed
`FsMkdir` SHALL create one directory whose parent exists (else `no such file or directory`; existing: `already exists`). `FsRename` SHALL move a file or a directory and MUST NOT overwrite: an existing `to` is refused with `<to>: already exists`, a missing `from` with `no such file or directory`. Both answer `FsAck`.

**Verified by**: `files::directories_are_made_entries_renamed_and_deleted`.

#### Scenario: Rename does not overwrite
- **WHEN** `final.md` exists and a client renames `other.md` to `final.md`
- **THEN** the error is `~/project/final.md: already exists` and `final.md` keeps its content

#### Scenario: Mkdir without a parent
- **WHEN** a client makes `~/a/b` and `~/a` does not exist
- **THEN** the error is `~/a/b: no such file or directory`

### Requirement: Entries are deleted
`FsDelete` SHALL remove a file, a symbolic link (not what it points to) or an empty directory, and with `recursive` a directory with everything in it; a non-empty directory without `recursive` MUST be refused with `directory not empty`. It answers `FsAck`.

**Verified by**: `files::directories_are_made_entries_renamed_and_deleted`.

#### Scenario: Recursive delete
- **WHEN** a client deletes the non-empty `~/moved` without `recursive`, then with it
- **THEN** the first is refused with `~/moved: directory not empty` and the second removes it

#### Scenario: A link goes, its target stays
- **WHEN** a client deletes `~/link` pointing to `moved/final.md`
- **THEN** the link is gone and `moved/final.md` is still a file

### Requirement: Watched directories report their changes
After `FsWatch` is acknowledged (`FsAck`; a non-directory: `not a directory`), each change to an entry of that directory, not of its subdirectories, SHALL be pushed as `FsChanged` with `request_id` 0, `watch` as named in the `FsWatch`, `path` absolute without links, and `kind` CREATE, MODIFY or REMOVE (REMOVE also for the watched directory itself). Changes to one entry within about 100 ms MUST come as one.

**Verified by**: `files::a_watched_directory_reports_what_changes_in_it`.

#### Scenario: Create, modify, remove
- **WHEN** a client watches `~/watched` and `new.txt` is created there, `old.txt` changed and `new.txt` removed
- **THEN** it gets `FsChanged` CREATE, MODIFY and REMOVE for those paths with `watch` `~/watched`

#### Scenario: Subdirectories
- **WHEN** a file is written in `~/watched/deeper` and a directory `made` is created in `~/watched`
- **THEN** only the creation of `made` is reported

### Requirement: Protocol saves are one change, and watches can stop
A save through `FsWrite` in a watched directory SHALL be reported as one MODIFY of the file, and the temporary file it went through MUST NOT be reported. A second `FsWatch` of the same path replaces the first. `FsUnwatch`, naming the path as in its watch, SHALL stop it and answer `FsAck`, or `<path>: not watched`. Watches end with the connection.

**Verified by**: `files::a_watched_directory_reports_what_changes_in_it`.

#### Scenario: Save through the protocol
- **WHEN** a client watching `~/watched` writes `~/watched/old.txt` with `FsWrite`
- **THEN** it gets one `FsChanged` MODIFY for `old.txt` and none for a `.tilr-write-` file

#### Scenario: After unwatching
- **WHEN** the client sends `FsUnwatch ~/watched` and a file is then created there
- **THEN** no `FsChanged` comes for it, and a second `FsUnwatch` answers `~/watched: not watched`

### Requirement: A read-only server changes nothing
With `[files] readonly = true` the server SHALL answer `FsList`, `FsStat`, `FsRead` and `FsWatch` as usual and MUST answer `FsWrite`, `FsMkdir`, `FsRename` and `FsDelete` with `FsAck.error` `files are read-only on this server`, changing nothing on disk.

**Verified by**: `files::a_read_only_server_shows_files_but_changes_nothing`.

#### Scenario: Write refused
- **WHEN** a read-only server gets an `FsWrite` of two chunks for `~/a.txt`
- **THEN** one `FsAck` with `files are read-only on this server` comes and `~/a.txt` is unchanged

## Notes

- Requirements without a test: none. Untested details inside requirements: the `not a regular file` refusal, a read failing when the file shrinks, a second `FsWatch` replacing the first, the startup warning for a missing root, and mode 0666-less-umask of a new file.
- The non-UTF-8 test `files::a_name_that_is_not_utf8_is_listed_with_its_bytes_and_named_by_them` runs on Linux only (macOS file systems take UTF-8 names only).
- No itests case cited here is currently red: UC-064, UC-071 and UC-072 were red before 2026-10-17 and pass at HEAD per itests/USE_CASES.md.
- Unit tests in `crates/tilrd/src/*.rs` are cited as `module::tests::name` with their file.
