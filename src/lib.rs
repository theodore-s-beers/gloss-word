#![warn(clippy::pedantic, clippy::nursery)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::uninlined_format_args
)]

use anyhow::Context;
use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use std::io::Write;
use std::process::{Command, Output, Stdio};
use std::sync::LazyLock;

static ETYMOLOGY_HEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\S)(\([a-z]{1,3}\.\))\n").unwrap());
static FIGURE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)\n\n!\[.+$").unwrap());
static NUMBERED_LIST_LABEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\n\*\*(?P<a>\d+\.)\*\*").unwrap());
static LETTERED_LIST_LABEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\n\*\*(?P<b>[a-z]\.)\*\*").unwrap());

/// The kind of dictionary information to retrieve and format.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LookupMode {
    Definition,
    Etymology,
}

impl LookupMode {
    #[must_use]
    pub fn lookup_url(self, word: &str) -> String {
        match self {
            Self::Definition => {
                format!(
                    "https://www.thefreedictionary.com/{}",
                    word.replace(' ', "+")
                )
            }
            Self::Etymology => {
                format!(
                    "https://www.etymonline.com/word/{}",
                    word.replace(' ', "%20")
                )
            }
        }
    }
}

#[must_use]
// Take list of elements and compile them into a string (as appropriate)
pub fn compile_results(mode: LookupMode, sections: Vec<ElementRef>) -> String {
    let mut results = String::new();

    match mode {
        LookupMode::Etymology => {
            // If etymology, just push everything from any sections
            for section in sections {
                results.push_str(&section.html());
            }
        }
        LookupMode::Definition => {
            // If definition, set up a few more selectors for desired elements
            let element_selectors = Selector::parse("div.pseg, h2, hr.hmsep").unwrap();

            // Push selected elements from first/only section
            if let Some(section) = sections.first() {
                for element in section.select(&element_selectors) {
                    results.push_str(&element.html());
                }
            }
        }
    }

    results
}

// Make HTTP request and read response body into string
pub fn get_response_text(lookup_url: &str) -> Result<String, anyhow::Error> {
    // sadly, we need a minimal browser-like UA to avoid Cloudflare bot challenges
    let client = reqwest::blocking::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh) AppleWebKit/605.1.15 (KHTML, like Gecko)")
        .build()
        .context("Failed to build HTTP client")?;

    let response_text = client
        .get(lookup_url)
        .send()
        .context("Failed to complete HTTP request")?
        .error_for_status()
        .context("HTTP request returned error status")?
        .text()
        .context("Failed to decode HTTP response body")?;

    Ok(response_text)
}

#[must_use]
// Cull certain elements from the HTML fragment, based on CSS selectors
pub fn get_sections(mode: LookupMode, parsed_chunk: &Html) -> Vec<ElementRef<'_>> {
    // Set up a selector for the relevant section
    let section_selector = match mode {
        LookupMode::Etymology => Selector::parse("h2.scroll-m-16 span, section.-mt-4").unwrap(),
        LookupMode::Definition => {
            Selector::parse(r#"div#Definition section[data-src="hm"]"#).unwrap()
        }
    };

    // Run the select iterator and collect the result(s) in a vec
    // For definition lookup, this should yield either one item, or nothing
    // For etymology lookup, it could yield multiple sections
    parsed_chunk.select(&section_selector).collect()
}

fn run_pandoc(input: &str, args: &[&str]) -> Result<Output, anyhow::Error> {
    let mut pandoc = Command::new("pandoc")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("Failed to start pandoc process")?;

    pandoc
        .stdin
        .as_mut()
        .context("Failed to open pandoc stdin")?
        .write_all(input.as_bytes())
        .context("Failed to write to pandoc stdin")?;

    let output = pandoc
        .wait_with_output()
        .context("Failed to read pandoc output")?;

    Ok(output)
}

// Function to convert to plain text with Pandoc, as a final step
// This used to be duplicated in pandoc_primary, but jscpd was complaining
pub fn pandoc_plain(input: &str, mode: LookupMode) -> Result<String, anyhow::Error> {
    let output = run_pandoc(input, &["-t", "plain"])?;
    let mut output_str = String::from_utf8_lossy(&output.stdout).into_owned();

    // In etym mode, insert space before POS in any headword line, if missing
    if mode == LookupMode::Etymology {
        output_str = ETYMOLOGY_HEADING
            .replace_all(&output_str, "$1 $2\n")
            .into_owned();
    }

    Ok(output_str)
}

// Main Pandoc function
pub fn pandoc_primary(results: &str, mode: LookupMode) -> Result<String, anyhow::Error> {
    // Take first Pandoc output as a string
    let output_1 = run_pandoc(
        results,
        &[
            "-f",
            "html+smart-native_divs",
            "-t",
            "markdown",
            "--wrap=none",
        ],
    )?;

    let output_str_1 = core::str::from_utf8(&output_1.stdout)?;

    // Make regex (and simple text) replacements, depending on search mode
    match mode {
        LookupMode::Etymology => {
            // Remove any figures
            let after_1 = FIGURE.replace_all(output_str_1, "");

            // Un-escape double quotes
            // I don't know why Pandoc is outputting these to begin with
            let after_2 = after_1.replace(r#"\\""#, r#"""#);

            pandoc_plain(&after_2, mode)
        }
        LookupMode::Definition => {
            // Un-bold numbered list labels
            let after_1 = NUMBERED_LIST_LABEL.replace_all(output_str_1, "\n$a");

            // Un-bold and indent lettered list labels
            let after_2 = LETTERED_LIST_LABEL.replace_all(&after_1, "\n    $b");

            // Un-escape double quotes
            let after_3 = after_2.replace(r#"\\""#, r#"""#);

            pandoc_plain(&after_3, mode)
        }
    }
}

// Convert fallback suggestions from HTML directly to plain text.
pub fn pandoc_fallback(results: &str) -> Result<String, anyhow::Error> {
    let output = run_pandoc(results, &["-f", "html+smart-native_divs", "-t", "plain"])?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[must_use]
// Take only part of the response text, for faster parsing
pub fn take_chunk(response_text: &str) -> Html {
    // In definition mode, we split the document
    // Otherwise we could blow a bunch of time parsing the whole thing
    // In etymology mode, this shouldn't do anything
    let chunk = response_text
        .split_once(r#"<div id="Thesaurus">"#)
        .map_or(response_text, |(chunk, _)| chunk);

    // Parse the first chunk, which is the one we want
    // For an etymology entry, the "first chunk" is the whole document
    Html::parse_fragment(chunk)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    fn full_sequence(mode: LookupMode, lookup_url: &str) -> Result<String, anyhow::Error> {
        let response_text = get_response_text(lookup_url)?;
        let parsed_chunk = take_chunk(&response_text);

        let sections = get_sections(mode, &parsed_chunk);
        if sections.is_empty() {
            return Err(anyhow!("Missing or bad response for the given URL"));
        }

        let results = compile_results(mode, sections);
        pandoc_primary(&results, mode)
    }

    #[test]
    fn lookup_urls_encode_spaces_for_the_selected_source() {
        assert_eq!(
            LookupMode::Definition.lookup_url("ice cream"),
            "https://www.thefreedictionary.com/ice+cream"
        );
        assert_eq!(
            LookupMode::Etymology.lookup_url("ice cream"),
            "https://www.etymonline.com/word/ice%20cream"
        );
    }

    #[test]
    fn def_atavism() {
        let lookup_url = "https://www.thefreedictionary.com/atavism";
        let output = full_sequence(LookupMode::Definition, lookup_url)
            .unwrap_or_else(|e| panic!("Test failed for {}: {}", lookup_url, e));

        let standard = "at·a·vism\n\nn.\n\n1.  The reappearance of a characteristic in an organism after several\n    generations of absence.\n\n2.  An individual or a part that exhibits atavism. Also called\n    throwback.\n\n3.  The return of a trait or recurrence of previous behavior after a\n    period of absence.\n";

        assert_eq!(output, standard);
    }

    #[test]
    fn def_isthmus() {
        std::thread::sleep(std::time::Duration::from_secs(5)); // Getting rate-limited in CI?

        let lookup_url = "https://www.thefreedictionary.com/isthmus";
        let output = full_sequence(LookupMode::Definition, lookup_url)
            .unwrap_or_else(|e| panic!("Test failed for {}: {}", lookup_url, e));

        let standard = "isth·mus\n\nn. pl. isth·mus·es or isth·mi (-mī′)\n\n1.  A narrow strip of land connecting two larger masses of land.\n\n2.  Anatomy\n\n    a.  A narrow strip of tissue joining two larger organs or parts of\n        an organ.\n\n    b.  A narrow passage connecting two larger cavities.\n";

        assert_eq!(output, standard);
    }

    #[test]
    fn etym_cummerbund() {
        let lookup_url = "https://www.etymonline.com/word/cummerbund";
        let output = full_sequence(LookupMode::Etymology, lookup_url)
            .unwrap_or_else(|e| panic!("Test failed for {}: {}", lookup_url, e));

        let standard = "cummerbund (n.)\n\n“large, loose sash worn as a belt,” 1610s, from Hindi kamarband “loin\nband,” from Persian kamar “waist” + band “something that ties,” from\nAvestan banda- “bond, fetter,” from PIE root *bhendh- “to bind.”\n";

        assert_eq!(output, standard);
    }

    #[test]
    fn etym_forest() {
        let lookup_url = "https://www.etymonline.com/word/forest";
        let output = full_sequence(LookupMode::Etymology, lookup_url)
            .unwrap_or_else(|e| panic!("Test failed for {}: {}", lookup_url, e));

        let standard = "forest (n.)\n\nlate 13c., “extensive tree-covered district,” especially one set aside\nfor royal hunting and under the protection of the king, from Old French\nforest “forest, wood, woodland” (Modern French forêt), probably\nultimately from Late Latin/Medieval Latin forestem silvam “the outside\nwoods,” a term from the Capitularies of Charlemagne denoting “the royal\nforest.” This word comes to Medieval Latin, perhaps via a Germanic\nsource akin to Old High German forst, from Latin foris “outside” (see\nforeign). If so, the sense is “beyond the park,” the park (Latin parcus;\nsee park (n.)) being the main or central fenced woodland.\n\nAnother theory traces it through Medieval Latin forestis, originally\n“forest preserve, game preserve,” from Latin forum in legal sense\n“court, judgment;” in other words “land subject to a ban” [Buck].\nReplaced Old English wudu (see wood (n.)). Spanish and Portuguese\nfloresta have been influenced by flor “flower.”\n\nforest (v.)\n\n“cover with trees or woods,” 1818 (forested is attested from 1610s),\nfrom forest (n.). The earlier word was afforest (c.\u{a0}1500).\n";

        assert_eq!(output, standard);
    }
}
