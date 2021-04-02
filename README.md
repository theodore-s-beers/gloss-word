# ahd-scrape

This program takes one argument: an English word to be looked up in the
[_American Heritage Dictionary_](https://en.wikipedia.org/wiki/The_American_Heritage_Dictionary_of_the_English_Language).
It then makes a request to the website of
[The Free Dictionary](https://www.thefreedictionary.com/), scrapes a few
relevant HTML elements, and prints those to `stdout`.

What use is this? Well, I have a shell function that pipes the output of
`ahd-scrape` to [Pandoc](https://github.com/jgm/pandoc), which converts the HTML
to nicely formatted plain text. I can use that function (pasted below) as a
little CLI dictionary utility.

In case I look up the same word more than once, rudimentary caching is
implemented. Successful results are saved as individual text files in the
directory `$HOME/.ahd-scrape/cache`. If there's a cache hit, the program returns
that saved HTML immediately, avoiding an HTTP request to TFD.

I'd like to take a more integrated approach, with the whole process handled by a
Rust binary that calls out to Pandoc (the one irreplaceable external dependency)
and returns plain text. But I haven't gotten there yet.

Answers to a few other potential questions: _Why scrape from TFD, as opposed to
other good dictionary sites?_ I actually tried Wiktionary first, but their
markup is not at all suited to this. _Why_ AHD, _as opposed to other English
dictionaries?_ I just like it. I looked at a few and chose the one that most
appealed to me.

## Shell function

```sh
gloss() {
  ahd-scrape "$1" |
    pandoc -f html+smart-native_divs -t markdown --wrap=none |
    sd '\n\*\*(\d+\.)\*\*' '\n$1' |
    sd '\n\*\*([a-z]\.)\*\*' '\n    $1' |
    pandoc -t plain |
    bat --style=grid,numbers
}
```

I've given this function the name `gloss`, so I can look up a word in my shell
by entering, for instance, `gloss filigree`. (See the asciicast below.) There
are a few dependencies other than `ahd-scrape`: Pandoc,
[`sd`](https://github.com/chmln/sd), and
[`bat`](https://github.com/sharkdp/bat). Only Pandoc (a famous Haskell library)
seems truly unavoidable. The other two are Rust libraries, which could easily be
integrated into `ahd-scrape` at some later point. `sd` is a partial
reimplementation of `sed`. I love it; I don't want to use `sed` ever again. (I
had a frustrating experience figuring out how to use `sed` on macOS, before
learning that it's derived from an old FreeBSD version whose syntax has key
differences from that of GNU `sed`. It left a bad taste in my mouth.) `bat` is a
clone of `cat`. It's totally optional, but I enjoy it.

HTML from `ahd-scrape` first goes to Pandoc to be converted into Markdown; then
to `sd` for a couple of modifications that facilitate list formatting; then back
to Pandoc to be converted into plain text; then to `bat` for pretty-printing.

## asciicast

[![asciicast](https://asciinema.org/a/Ir3YZLmzEZxuFxZTzsnjeCjZ1.svg)](https://asciinema.org/a/Ir3YZLmzEZxuFxZTzsnjeCjZ1)
