# p2mcli

Disassembler and linter for Patrician 3 mission scripts (`.p2m`), the letter-script
bytecode the interpreter at `0x004ECF64` runs. The format and the opcode table are in the
gitbook under *File Formats / Mission Scripts (.p2m)*.

The scripts live under `missions_addon/` inside `p2arch0_eng.cpr` and `p2arch1_eng.cpr`;
extract them with `cprcli` first.

## Disassemble

```
cargo run --bin p2mcli -- dis "unpacked/p2arch0_eng/missions_addon/patrouille.p2m"
```

One line per command - index, offset, raw bytes, decoded form - then the string pool.
Known opcodes are decoded; unknown ones print their bytes with `???`. A command whose
length from the offset table differs from the decoder's expectation is flagged
(`<< len`), which is how new opcode lengths get established.

## Lint

```
cargo run --bin p2mcli -- lint "unpacked/*/missions_addon/*.p2m"
```

Reports the patrol-letter class of bug: a create-letter command (`F7`) whose town
operand names a variable that no command writes a town into. For every `F7` it lists the
town variable and every command that writes it, classified by the kind of value the
command produces; a letter whose town variable is only written by non-town producers, is
out of range, or is never written at all, is the bug.
