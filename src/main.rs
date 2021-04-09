use std::fs::File;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Command;
use std::{fs, str};

use clap::{crate_version, App, Arg};
use directories::ProjectDirs;
use isahc::prelude::*;
use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use tempfile::NamedTempFile;

fn main() -> Result<(), isahc::Error> {
    // Set up CLI
    let matches = App::new("gloss-word")
        .version(crate_version!())
        .author("Theo Beers <theo.beers@fu-berlin.de>")
        .about("A simple CLI dictionary")
        .arg(
            Arg::with_name("etym")
                .short("e")
                .long("etymology")
                .help("Search for etymology instead of definition"),
        )
        .arg(
            Arg::with_name("INPUT")
                .help("The word or phrase to look up")
                .required(true)
                .index(1),
        )
        .get_matches();

    // Do we have the etymology flag?
    let etym_mode: bool = matches.is_present("etym");

    // Take input and lowercase it
    // Is this ok to unwrap, or should I switch to expect?
    let desired_word = matches.value_of("INPUT").unwrap().to_lowercase();

    // Do we even have the relevant cache subdir? False by default
    let mut has_cache_subdir = false;

    // Set up a PathBuf for a possible path, and a filename
    let mut notional_file_path = PathBuf::new();
    let notional_filename = desired_word.clone() + ".txt";

    // Try at least to get to the point of having the cache subdir available
    if let Some(proj_dirs) = ProjectDirs::from("com", "theobeers", "gloss-word") {
        let cache_dir = proj_dirs.cache_dir();

        let mut cache_subdir = PathBuf::from(cache_dir);

        // Set relevant subdir name
        if etym_mode {
            cache_subdir.push("etym")
        } else {
            cache_subdir.push("def")
        }

        if cache_subdir.exists() {
            // Already had cache subdir
            has_cache_subdir = true;

            notional_file_path.push(cache_subdir);
            notional_file_path.push(notional_filename);
        } else {
            // Else try to create it
            let create_subdir = fs::create_dir_all(&cache_subdir);

            if create_subdir.is_ok() {
                has_cache_subdir = true;

                notional_file_path.push(cache_subdir);
                notional_file_path.push(notional_filename);
            }
        }
    }

    // If the appropriate file path actually exists, try to read it
    if has_cache_subdir && notional_file_path.exists() {
        let try_open = File::open(&notional_file_path);

        if let Ok(mut file) = try_open {
            let mut contents = String::new();
            let try_read = file.read_to_string(&mut contents);

            if try_read.is_ok() {
                // Success? Print file contents and return
                print!("{}", contents);

                return Ok(());
            }
        }
    }

    // If we're moving on, it means we didn't get a cached result
    // That's fine; it's going to be the default case

    // Assemble URL

    let mut lookup_url: String;

    if etym_mode {
        lookup_url = "https://www.etymonline.com/word/".to_string();
        lookup_url += &desired_word.replace(" ", "%20");
    } else {
        lookup_url = "https://www.thefreedictionary.com/".to_string();
        lookup_url += &desired_word.replace(" ", "+");
    }

    // Make the request
    let mut response = isahc::get(lookup_url)?;

    // Get the document text
    let response_text = response.text()?;

    // Set up a regex to split the document, in definition mode
    // In etymology mode, this shouldn't do anything
    let re_thesaurus = Regex::new(r#"<div id="Thesaurus">"#).unwrap();

    // Split document (if applicable)
    // Otherwise we could blow a bunch of time parsing the whole thing
    let chunks: Vec<&str> = re_thesaurus.split(&response_text).collect();

    // Parse the first chunk, which is the one we want
    // For an etymology entry, the "first chunk" is the whole document
    let parsed_chunk = Html::parse_fragment(chunks[0]);

    // Set up a selector for the relevant section
    let section_selector = match etym_mode {
        true => Selector::parse(r#"div[class^="word--"]"#).unwrap(),
        _ => Selector::parse(r#"div#Definition section[data-src="hm"]"#).unwrap(),
    };

    // Run the select iterator and collect the result(s) in a vec
    // For definition lookup, this should yield either one item, or nothing
    // For etymology lookup, it could yield multiple sections
    let section_vec: Vec<ElementRef> = parsed_chunk.select(&section_selector).collect();

    // Set up a few more selectors for desired elements (in definition mode)
    let element_selectors = Selector::parse("div.pseg, h2, hr.hmsep").unwrap();

    // Check to see if we got the section we wanted
    if !section_vec.is_empty() {
        // Set up a string to hold results
        let mut results = String::new();

        if etym_mode {
            // If etymology, just push everything from any sections
            for item in section_vec.iter() {
                results.push_str(&item.html());
            }
        } else {
            // If definition, push selected elements from first/only section
            for element in section_vec[0].select(&element_selectors) {
                results.push_str(&element.html());
            }
        }

        // Update: now we're calling out to Pandoc from the Rust program
        // It still requires two runs with regex replacement in between
        // I'm using tempfiles to feed input to Pandoc
        // At the end, we should have a nice plain text file to cache

        let mut final_output = String::new();

        if etym_mode {
            let mut pandoc_input = NamedTempFile::new().expect("Failed to create tempfile");
            write!(pandoc_input, "{}", results).expect("Failed to write to tempfile");

            let pandoc_1 = Command::new("pandoc")
                .arg(pandoc_input.path())
                .arg("-f")
                .arg("html+smart-native_divs")
                .arg("-t")
                .arg("markdown")
                .arg("--wrap=none")
                .output()
                .expect("Failed to execute Pandoc");

            let output_1 = str::from_utf8(&pandoc_1.stdout)
                .expect("Failed to convert Pandoc output to string");

            let re_quotes = Regex::new(r#"\\""#).unwrap();
            let after = re_quotes.replace_all(&output_1, r#"""#).to_string();

            let mut input_file_2 = NamedTempFile::new().expect("Failed to create tempfile");
            write!(input_file_2, "{}", after).expect("Failed to write to tempfile");

            let pandoc_2 = Command::new("pandoc")
                .arg(input_file_2.path())
                .arg("-t")
                .arg("plain")
                .output()
                .expect("Failed to execute Pandoc");

            let output_2 = str::from_utf8(&pandoc_2.stdout)
                .expect("Failed to convert Pandoc output to string");

            final_output.push_str(output_2);
        } else {
            let mut pandoc_input = NamedTempFile::new().expect("Failed to create tempfile");
            write!(pandoc_input, "{}", results).expect("Failed to write to tempfile");

            let pandoc_1 = Command::new("pandoc")
                .arg(pandoc_input.path())
                .arg("-f")
                .arg("html+smart-native_divs")
                .arg("-t")
                .arg("markdown")
                .arg("--wrap=none")
                .output()
                .expect("Failed to execute Pandoc");

            let output_1 = str::from_utf8(&pandoc_1.stdout)
                .expect("Failed to convert Pandoc output to string");

            let re_list_1 = Regex::new(r"\n\*\*(?P<a>\d+\.)\*\*").unwrap();
            let after_1 = re_list_1.replace_all(&output_1, "\n$a").to_string();

            let re_list_2 = Regex::new(r"\n\*\*(?P<b>[a-z]\.)\*\*").unwrap();
            let after_2 = re_list_2.replace_all(&after_1, "\n    $b").to_string();

            let mut input_file_2 = NamedTempFile::new().expect("Failed to create tempfile");
            write!(input_file_2, "{}", after_2).expect("Failed to write to tempfile");

            let pandoc_2 = Command::new("pandoc")
                .arg(input_file_2.path())
                .arg("-t")
                .arg("plain")
                .output()
                .expect("Failed to execute Pandoc");

            let output_2 = str::from_utf8(&pandoc_2.stdout)
                .expect("Failed to convert Pandoc output to string");

            final_output.push_str(output_2);
        }

        // If the cache subdir proved available earlier, try to create a file
        if has_cache_subdir {
            let try_file = File::create(&notional_file_path);

            // If we have the new file, write the results into it
            // Again, errors are just ignored
            if let Ok(mut file) = try_file {
                let _ = file.write_all(final_output.as_bytes());
            };
        }

        // We still need to print results, of course
        print!("{}", final_output);
    } else {
        // If we didn't get an etymology result, stop here
        if etym_mode {
            panic!("Etymology not found");
        }

        // In dictionary mode, we can check for a list of similar words
        let suggestions_selector = Selector::parse("ul.suggestions li").unwrap();
        let suggestions_vec: Vec<ElementRef> = parsed_chunk.select(&suggestions_selector).collect();

        // Again, see if we got anything
        if !suggestions_vec.is_empty() {
            // If so, collect results
            let mut results = String::new();

            for element in suggestions_vec.iter() {
                results.push_str(&element.html());
            }

            // Put them through Pandoc quickly to get clean plain text
            let mut pandoc_input = NamedTempFile::new().expect("Failed to create tempfile");
            write!(pandoc_input, "{}", results).expect("Failed to write to tempfile");

            let pandoc_1 = Command::new("pandoc")
                .arg(pandoc_input.path())
                .arg("-f")
                .arg("html+smart-native_divs")
                .arg("-t")
                .arg("plain")
                .output()
                .expect("Failed to execute Pandoc");

            let output_1 = str::from_utf8(&pandoc_1.stdout)
                .expect("Failed to convert Pandoc output to string");

            // Print an explanatory message, then the results
            print!("Did you mean:\n\n");
            print!("{}", output_1);
        } else {
            // If still no dice, panic
            panic!("Definition not found");
        }
    }

    Ok(())
}
