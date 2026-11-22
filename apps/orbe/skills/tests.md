# orbe: tests

Load this when a test does more than the example in `orbe skills get core`.

A test is `tests/<name>.test`. Each line is a live command without `orbe`. `#` starts a comment. `"..."` holds spaces. Inside quotes only `\"` and `\\` escape, so a test cannot name a line ending. To tell lines apart, print text only one line has, such as `led=on`. `open` alone opens `scene.jsonc`.

`orbe test` runs every `tests/*.test`, each in a fresh session, and stops a file at its first failed line. `orbe test <file or folder>…` runs only those. `orbe test -v` prints each command and its reply. The file's session ends with `orbe test`. A session from `orbe open` keeps running, untouched. `orbe test` takes no `--session` and ignores `ORBE_SESSION`.

`--json`, `--session`, `--help` or `-h` on a line is `usage`, except as the text after `serial contains`. Live, `--help` anywhere prints help, so `orbe expect serial contains --help` prints help, while the same test line looks for that text. A test file refuses `play` and `pause` with `usage`.

`expect` and `step --until` take the same conditions. Quote `<`, `>` and a colour in a shell.

- `@led1 lit`, `@led1 dim`, `@led1 off`. Load `orbe skills get ratings` for the current each state needs.
- `@nano.D13 high`, `@nano.D13 low`.
- `@led1 rgb(255,0,0)`, an addressable LED's latched colour. `#` starts a comment, so a colour is never hex. In a shell, `orbe expect @led1 'rgb(255,0,0)'`.
- `@led1.i > 5mA`, `@nano.D2 < 1V`, with `<`, `<=`, `>` or `>=`. In a shell, `orbe expect @led1.i '>' 5mA`.
- `serial contains "text"`, all the serial text since `open`. `snapshot` shows the last 10 lines, joined with `\n`.
- `no diagnostics`. A chip warning can show up to one engine step (`sim.max_step`, 10 ms by default) late, so step that far past its cause first.

A failure prints the line, what was expected and what it saw, then `0 passed, 1 failed`.

```bash
# run one file and show each reply
orbe test -v tests/button.test
```
