
Default index writes preserve decoded resolve-undo (`REUC`) records, including missing conflict stages.
`Extensions::None` and `Extensions::Given` omit resolve-undo; use the default `Extensions::All` to retain it.
Decoded V4 indexes retain prefix compression when written. Decoding can budget the entry vector
separately with `entry_alloc_limit_bytes`, without relaxing the path or extension allocation limits.

#### Test fixtures

Most of the test indices are snatched directly from the unit test suite of `git` itself, usually by running something like the following

```shell
 ./t1700-split-index.sh -r 2 --debug 
```

Then one finds all test state and the index in particular in `trash directory/t1700-split-index/.git/index` and can possibly copy it over and use as fixture.
The preferred way is to find a test of interest, and use its setup code within one of our own fixture scripts that are executed once to generate the file of interest.
