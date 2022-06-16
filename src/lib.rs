use std::io::Write;
use std::process::Command;
use std::str;

use anyhow::Context;
use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use tempfile::NamedTempFile;

// Take list of elements and compile them into a string (as appropriate)
pub fn compile_results(etym_mode: bool, section_vec: Vec<ElementRef>) -> String {
    // Set up a string to hold results
    let mut results = String::new();

    if etym_mode {
        // If etymology, just push everything from any sections
        for section in section_vec.iter() {
            results.push_str(&section.html());
        }
    } else {
        // If definition, set up a few more selectors for desired elements
        let element_selectors = Selector::parse("div.pseg, h2, hr.hmsep").unwrap();

        // Push selected elements from first/only section
        for element in section_vec[0].select(&element_selectors) {
            results.push_str(&element.html());
        }
    }

    results
}

// Make HTTP request and read response body into string
pub fn get_response_text(lookup_url: String) -> Result<String, anyhow::Error> {
    let response_text = reqwest::blocking::get(lookup_url)
        .context("Failed to complete HTTP request")?
        .text()
        .context("Failed to decode HTTP response body")?;

    Ok(response_text)
}

// Cull certain elements from the HTML fragment, based on CSS selectors
pub fn get_section_vec(etym_mode: bool, parsed_chunk: &Html) -> Vec<ElementRef> {
    // Set up a selector for the relevant section
    let section_selector = match etym_mode {
        true => Selector::parse(r#"div[class^="word--"]:not([class*="word_4pc"])"#).unwrap(),
        _ => Selector::parse(r#"div#Definition section[data-src="hm"]"#).unwrap(),
    };

    // Run the select iterator and collect the result(s) in a vec
    // For definition lookup, this should yield either one item, or nothing
    // For etymology lookup, it could yield multiple sections
    let section_vec: Vec<ElementRef> = parsed_chunk.select(&section_selector).collect();

    section_vec
}

// Function to convert to plain text with Pandoc, as a final step
// This used to be duplicated in pandoc_primary, but jscpd was complaining
pub fn pandoc_plain(input: String) -> Result<String, anyhow::Error> {
    // String is again written to a tempfile for Pandoc
    let mut input_file = NamedTempFile::new().context("Failed to create tempfile")?;
    write!(input_file, "{}", input).context("Failed to write to tempfile")?;

    let pandoc = Command::new("pandoc")
        .arg(input_file.path())
        .arg("-t")
        .arg("plain")
        .output()
        .context("Failed to execute Pandoc")?;

    let output = str::from_utf8(&pandoc.stdout)
        .context("Failed to convert Pandoc output to string")?
        .to_string();

    Ok(output)
}

// Main Pandoc function
pub fn pandoc_primary(etym_mode: bool, results: String) -> Result<String, anyhow::Error> {
    #[allow(clippy::needless_late_init)]
    let final_output: String;

    // Write results string into a tempfile to pass to Pandoc
    let mut input_file_1 = NamedTempFile::new().context("Failed to create tempfile")?;
    write!(input_file_1, "{}", results).context("Failed to write to tempfile")?;

    let pandoc_1 = Command::new("pandoc")
        .arg(input_file_1.path())
        .arg("-f")
        .arg("html+smart-native_divs")
        .arg("-t")
        .arg("markdown")
        .arg("--wrap=none")
        .output()
        .context("Failed to execute Pandoc")?;

    // Take first Pandoc output as a string
    let output_1 =
        str::from_utf8(&pandoc_1.stdout).context("Failed to convert Pandoc output to string")?;

    // Make regex (and simple text) replacements, depending on search mode
    if etym_mode {
        // This is to remove any figures
        let re_figures = Regex::new(r"(?m)\n\n!\[.+$").unwrap();
        let after_1 = re_figures.replace_all(output_1, "");

        // This just un-escapes double quotes
        // Don't know why Pandoc is outputting these, anyway
        let after_2 = after_1.replace(r#"\\""#, r#"""#);

        // Get final output
        final_output = pandoc_plain(after_2)?;
    } else {
        // Un-bold numbered list labels
        let re_list_1 = Regex::new(r"\n\*\*(?P<a>\d+\.)\*\*").unwrap();
        let after_1 = re_list_1.replace_all(output_1, "\n$a");

        // Un-bold and indent lettered list labels
        let re_list_2 = Regex::new(r"\n\*\*(?P<b>[a-z]\.)\*\*").unwrap();
        let after_2 = re_list_2.replace_all(&after_1, "\n    $b");

        // This just un-escapes double quotes
        // Don't know why Pandoc is outputting these, anyway
        let after_3 = after_2.replace(r#"\\""#, r#"""#);

        // Get final output
        final_output = pandoc_plain(after_3)?;
    }

    Ok(final_output)
}

// Take only part of the response text, for faster parsing
pub fn take_chunk(response_text: String) -> Html {
    // In definition mode, we split the document
    // Otherwise we could blow a bunch of time parsing the whole thing
    // In etymology mode, this shouldn't do anything
    let chunks: Vec<&str> = response_text.split(r#"<div id="Thesaurus">"#).collect();

    // Parse the first chunk, which is the one we want
    // For an etymology entry, the "first chunk" is the whole document
    Html::parse_fragment(chunks[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_sequence(etym_mode: bool, lookup_url: String) -> String {
        let response_text = get_response_text(lookup_url).unwrap();

        let parsed_chunk = take_chunk(response_text);

        let section_vec = get_section_vec(etym_mode, &parsed_chunk);

        let results = compile_results(etym_mode, section_vec);

        let output = pandoc_primary(etym_mode, results).unwrap();

        output
    }

    #[test]
    fn def_atavism() {
        let etym_mode = false;
        let lookup_url = String::from("https://www.thefreedictionary.com/atavism");

        let output = full_sequence(etym_mode, lookup_url);

        let standard = String::from("at·a·vism\n\nn.\n\n1.  The reappearance of a characteristic in an organism after several\n    generations of absence.\n\n2.  An individual or a part that exhibits atavism. Also called\n    throwback.\n\n3.  The return of a trait or recurrence of previous behavior after a\n    period of absence.\n");

        assert_eq!(output, standard);
    }

    #[test]
    fn def_isthmus() {
        let etym_mode = false;
        let lookup_url = String::from("https://www.thefreedictionary.com/isthmus");

        let output = full_sequence(etym_mode, lookup_url);

        let standard = String::from("isth·mus\n\nn. pl. isth·mus·es or isth·mi (-mī′)\n\n1.  A narrow strip of land connecting two larger masses of land.\n\n2.  Anatomy\n\n    a.  A narrow strip of tissue joining two larger organs or parts of\n        an organ.\n\n    b.  A narrow passage connecting two larger cavities.\n");

        assert_eq!(output, standard);
    }

    #[test]
    fn etym_cummerbund() {
        let etym_mode = true;
        let lookup_url = String::from("https://www.etymonline.com/word/cummerbund");

        let output = full_sequence(etym_mode, lookup_url);

        let standard = String::from("cummerbund (n.)\n\n“large, loose sash worn as a belt,” 1610s, from Hindi kamarband “loin\nband,” from Persian kamar “waist” + band “something that ties,” from\nAvestan banda- “bond, fetter,” from PIE root *bhendh- “to bind.”\n");

        assert_eq!(output, standard);
    }

    #[test]
    fn etym_forest() {
        let etym_mode = true;
        let lookup_url = String::from("https://www.etymonline.com/word/forest");

        let output = full_sequence(etym_mode, lookup_url);

        let standard = String::from("forest (n.)\n\nlate 13c., “extensive tree-covered district,” especially one set aside\nfor royal hunting and under the protection of the king, from Old French\nforest “forest, wood, woodland” (Modern French forêt), probably\nultimately from Late Latin/Medieval Latin forestem silvam “the outside\nwoods,” a term from the Capitularies of Charlemagne denoting “the royal\nforest.” This word comes to Medieval Latin, perhaps via a Germanic\nsource akin to Old High German forst, from Latin foris “outside” (see\nforeign). If so, the sense is “beyond the park,” the park (Latin parcus;\nsee park (n.)) being the main or central fenced woodland.\n\nAnother theory traces it through Medieval Latin forestis, originally\n“forest preserve, game preserve,” from Latin forum in legal sense\n“court, judgment;” in other words “land subject to a ban” [Buck].\nReplaced Old English wudu (see wood (n.)). Spanish and Portuguese\nfloresta have been influenced by flor “flower.”\n\nforest (v.)\n\n“cover with trees or woods,” 1818 (forested is attested from 1610s),\nfrom forest (n.). The earlier word was afforest (c.\u{a0}1500).\n");

        assert_eq!(output, standard);
    }
}
