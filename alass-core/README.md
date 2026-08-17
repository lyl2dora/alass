# alass-core

This Rust library contains the core algorithm for `alass`, the "Automatic Language-Agnostic Subtitle Sychronization" tool. If you want to go to the command line tool instead, please click [here](https://github.com/kaegi/alass).


## How to use the library

This fork is not published, so depend on it by path or by git:

```toml
[dependencies]
alass-core = { git = "https://github.com/kaegi/alass" }
```

The two entry points take two sequences of time spans and return the offset for each span of the
second sequence: `align` finds the best alignment allowing breaks, `align_nosplit` shifts
everything by a single offset. `get_nosplit_score` rates a result, and `standard_scoring` /
`overlap_scoring` are the two scoring functions to hand them.

The [2019 release on crates.io](https://crates.io/crates/alass-core) predates this fork.

### Documentaion

For much more information, please see the workspace information [here](https://github.com/kaegi/alass).