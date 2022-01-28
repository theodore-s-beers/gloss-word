use std::io::Write;
use std::process::Command;
use std::str;

use anyhow::Context;
use isahc::prelude::*;
use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use tempfile::NamedTempFile;

// Write new comment to explain this
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

// Write new comment to explain this
pub fn get_response_text(lookup_url: String) -> Result<String, anyhow::Error> {
    let response_text = isahc::get(lookup_url)
        .context("Failed to complete HTTP request")?
        .text()
        .context("Failed to read HTTP response body to string")?;

    Ok(response_text)
}

// Write new comment to explain this
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

// Function to call Pandoc in case of suggested alternate words
pub fn pandoc_fallback(results: String) -> Result<String, anyhow::Error> {
    // Write results string into a tempfile to pass to Pandoc
    let mut pandoc_input = NamedTempFile::new().context("Failed to create tempfile")?;
    write!(pandoc_input, "{}", results).context("Failed to write to tempfile")?;

    let pandoc = Command::new("pandoc")
        .arg(pandoc_input.path())
        .arg("-f")
        .arg("html+smart-native_divs")
        .arg("-t")
        .arg("plain")
        .output()
        .context("Failed to execute Pandoc")?;

    let pandoc_output = str::from_utf8(&pandoc.stdout)
        .context("Failed to convert Pandoc output to string")?
        .to_string();

    Ok(pandoc_output)
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

// Write new comment to explain this
pub fn take_chunk(response_text: String) -> Html {
    // In definition mode, we split the document
    // Otherwise we could blow a bunch of time parsing the whole thing
    // In etymology mode, this shouldn't do anything
    let chunks: Vec<&str> = response_text.split(r#"<div id="Thesaurus">"#).collect();

    // Parse the first chunk, which is the one we want
    // For an etymology entry, the "first chunk" is the whole document
    Html::parse_fragment(chunks[0])
}
