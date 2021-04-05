# ahd-scrape

This program takes one argument: an English word to be looked up in the
[_American Heritage Dictionary_](https://en.wikipedia.org/wiki/The_American_Heritage_Dictionary_of_the_English_Language).
It then makes a request (if necessary) to the website of
[The Free Dictionary](https://www.thefreedictionary.com/), scrapes a few
relevant HTML elements, converts that material to plain text with
[Pandoc](https://github.com/jgm/pandoc), and prints it to `stdout`. Results are
cached in a rudimentary manner, so that repeat searches—however unlikely they
may be—will not require fetching from TFD.

`ahd-scrape` functions, then, as a little CLI dictionary utility. Rather than
calling it directly, I use a shell function (pasted below) that pipes the output
to [`bat`](https://github.com/sharkdp/bat) for pretty-printing. But this is just
a matter of preference.

Pandoc is a required external dependency. Everything else is handled by the Rust
binary. I should note, however, that I wrote this program for my own use on
macOS, and I haven't yet tested it on Windows or Linux. I'm sure that some
adjustments will be necessary.

Cached results are in the form of individual text files in the directory
`$HOME/.ahd-scrape/cache`.

Answers to a few other potential questions: _Why scrape from TFD, as opposed to
other good dictionary sites?_ I actually tried Wiktionary first, but their
markup is not at all suited to this. _Why_ AHD, _as opposed to other English
dictionaries?_ I just like it. I looked at a few and chose the one that most
appealed to me.

## Shell function

```sh
gloss() {
  ahd-scrape "$1" | bat --style=grid,numbers
}
```

I've given this function the name `gloss`, so I can look up a word in my shell
by entering, for instance, `gloss filigree`. (See the asciicast below.) Again,
Pandoc is an external dependency of `ahd-scrape`; and `bat` is a Rust
quasi-reimplementation of `cat`, which I like to use for pretty-printing.

## asciicast

[![asciicast](https://asciinema.org/a/Ir3YZLmzEZxuFxZTzsnjeCjZ1.svg)](https://asciinema.org/a/Ir3YZLmzEZxuFxZTzsnjeCjZ1)
