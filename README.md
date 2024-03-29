# gloss-word

This is a simple CLI utility for looking up the definition or (with flag `-e`)
the etymology of an English word. Definitions are drawn from the
[_American Heritage Dictionary_](https://en.wikipedia.org/wiki/The_American_Heritage_Dictionary_of_the_English_Language),
as provided by the website of
[The Free Dictionary](https://www.thefreedictionary.com/). Etymologies are
pulled from the [Online Etymology Dictionary](https://www.etymonline.com/).

While the package is called `gloss-word`—since I needed a unique name to publish
it to [crates.io](https://crates.io/crates/gloss-word)—the binary itself, and
hence the command, is `gloss`. I also like to use a shell function (pasted
below) to pipe the output to [bat](https://github.com/sharkdp/bat) for
pretty-printing.

In short, the program makes a request (if necessary) to the appropriate website;
scrapes relevant HTML elements; converts that material to nicely formatted plain
text with [Pandoc](https://github.com/jgm/pandoc); and prints it to `stdout`.
Results are cached in a rudimentary manner, so that repeat searches—however
unlikely they may be—will not require fetching from TFD or Etymonline.

**Pandoc is a required external dependency.** Everything else is handled by the
Rust binary. I should note, however, that I wrote this program initially for my
own use on macOS, and I've tested it only lightly on Windows (seems fine), and
not at all on Linux (though feedback from other users suggests no problems). Bug
reports relating to OS compatibility would be welcome.

Cached results are in the form of a basic SQLite database, in what is supposed
to be a platform-appropriate location (relying on the
[directories](https://github.com/dirs-dev/directories-rs) library).

Answers to a few other potential questions: _Why scrape from TFD, as opposed to
other good dictionary sites?_ I actually tried Wiktionary first, but their
markup is not at all suited to this. _Why_ AHD, _as opposed to other English
dictionaries?_ I just like it. I looked at a few and chose the one that most
appealed to me.

## Shell function

```sh
gloss() {
  (
    set -o pipefail
    command gloss "$@" | bat --style=grid,numbers
  )
}
```

[bat](https://github.com/sharkdp/bat) is a Rust quasi-reimplementation of `cat`,
which I enjoy. You might also like to give it a try.

## asciicast

[![asciicast](https://asciinema.org/a/K8Dp5YncS2qVRL9965ayESTDC.svg)](https://asciinema.org/a/K8Dp5YncS2qVRL9965ayESTDC)
