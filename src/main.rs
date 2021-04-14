use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::{fs, str};

use clap::{crate_version, App, Arg};
use directories::ProjectDirs;
use indicatif::{ProgressBar, ProgressStyle};
use isahc::prelude::*;
use regex::Regex;
use rusqlite::{Connection, Result};
use scraper::{ElementRef, Html, Selector};
use tempfile::NamedTempFile;

#[derive(Debug)]
struct Entry {
    word: String,
    content: String,
}

fn main() -> Result<(), isahc::Error> {
    //
    // CLI SETUP
    //

    let matches = App::new("gloss-word")
        .version(crate_version!())
        .author("Theo Beers <theo.beers@fu-berlin.de>")
        .about("A simple English dictionary lookup utility")
        .arg(
            Arg::with_name("etymology")
                .short("e")
                .long("etymology")
                .help("Search for etymology instead of definition"),
        )
        .arg(
            Arg::with_name("fetch-update")
                .short("f")
                .long("fetch-update")
                .help("Fetch new data; update cache if applicable"),
        )
        .arg(
            Arg::with_name("INPUT")
                .help("The word or phrase to look up")
                .required(true)
                .index(1),
        )
        .get_matches();

    //
    // GLOBAL VARIABLES
    //

    // Do we have flags?
    let etym_mode: bool = matches.is_present("etymology");
    let force_fetch = matches.is_present("fetch-update");

    // Take input and lowercase it
    // Is this ok to unwrap, or should I switch to expect?
    let desired_word = matches.value_of("INPUT").unwrap().to_lowercase();

    // Is the cache db available? Also set up a variable for its path
    let mut db_available = false;
    let mut db_path = PathBuf::new();

    // Did we get a cache hit?
    let mut cache_hit = false;

    //
    // CHECK FOR CACHED RESULT
    //

    // In this whole section, errors tend to be ignored
    // Failure will just means fetching from the relevant website

    // Try at least to get to the point of having the db available
    if let Some(proj_dirs) = ProjectDirs::from("com", "theobeers", "gloss-word") {
        // This should give us a platform-appropriate cache directory
        let cache_dir = proj_dirs.cache_dir();

        // If we don't have the cache dir yet, try to create it
        if !cache_dir.exists() {
            let _ = fs::create_dir_all(cache_dir);
        }

        // Construct appropriate path for db
        db_path.push(cache_dir);
        db_path.push("entries.sqlite");

        // Create and/or connect to db; create table if needed
        if let Ok(db_conn) = Connection::open(&db_path) {
            // Handle etymology mode first
            if etym_mode {
                if db_conn
                    .execute(
                        "CREATE TABLE IF NOT EXISTS etymology (
                                word        TEXT UNIQUE NOT NULL,
                                content     TEXT NOT NULL
                            )",
                        [],
                    )
                    .is_ok()
                {
                    // Indicate that db is available (for later, if needed)
                    db_available = true;

                    // Set up query for possible cached result
                    let mut query = String::from("SELECT * FROM etymology WHERE word = '");
                    query.push_str(&desired_word);
                    query.push('\'');

                    // Run the query
                    if let Ok(mut stmt) = db_conn.prepare(&query) {
                        if let Ok(entry_iter) = stmt.query_map([], |row| {
                            Ok(Entry {
                                word: row.get(0)?,
                                content: row.get(1)?,
                            })
                        }) {
                            // If we got something, print and return (probably)
                            if let Some(actual_entry) = entry_iter.flatten().next() {
                                cache_hit = true;

                                if !force_fetch {
                                    print!("{}", actual_entry.content);
                                    return Ok(());
                                }
                            }
                        }
                    }
                }
            // Else the default is dictionary mode
            } else if db_conn
                .execute(
                    "CREATE TABLE IF NOT EXISTS dictionary (
                            word        TEXT UNIQUE NOT NULL,
                            content     TEXT NOT NULL
                        )",
                    [],
                )
                .is_ok()
            {
                // Indicate that db is available (for later, if needed)
                db_available = true;

                // Set up query for possible cached result
                let mut query = String::from("SELECT * FROM dictionary WHERE word = '");
                query.push_str(&desired_word);
                query.push('\'');

                // Run the query
                if let Ok(mut stmt) = db_conn.prepare(&query) {
                    if let Ok(entry_iter) = stmt.query_map([], |row| {
                        Ok(Entry {
                            word: row.get(0)?,
                            content: row.get(1)?,
                        })
                    }) {
                        // If we got something, print and return (probably)
                        if let Some(actual_entry) = entry_iter.flatten().next() {
                            cache_hit = true;

                            if !force_fetch {
                                print!("{}", actual_entry.content);
                                return Ok(());
                            }
                        }
                    }
                }
            }
        }
    }

    // If we're moving on, it means we didn't get a cached result
    // That's fine; it's going to be the default case

    //
    // SCRAPING & CACHING
    //

    // Build the relevant URL
    let mut lookup_url: String;

    if etym_mode {
        lookup_url = "https://www.etymonline.com/word/".to_string();
        lookup_url += &desired_word.replace(" ", "%20");
    } else {
        lookup_url = "https://www.thefreedictionary.com/".to_string();
        lookup_url += &desired_word.replace(" ", "+");
    }

    // Start a progress spinner
    let pb = ProgressBar::new_spinner();
    pb.enable_steady_tick(80);
    pb.set_style(
        ProgressStyle::default_spinner()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
            .template("{spinner} {msg}"),
    );
    pb.set_message("Fetching...");

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

    // Check to see if we got the section(s) we wanted
    if !section_vec.is_empty() {
        // Set up a string to hold results
        let mut results = String::new();

        if etym_mode {
            // If etymology, just push everything from any sections
            for section in section_vec.iter() {
                results.push_str(&section.html());
            }
        } else {
            // If definition, push selected elements from first/only section
            for element in section_vec[0].select(&element_selectors) {
                results.push_str(&element.html());
            }
        }

        // Update: now we're calling out to Pandoc from the Rust program
        // It still requires two runs, with regex replacement in between
        // Tempfiles are used to feed input to Pandoc

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
            let after_1 = re_quotes.replace_all(&output_1, r#"""#).to_string();

            let re_figures = Regex::new(r#"(?m)\n\n!\[.+$"#).unwrap();
            let after_2 = re_figures.replace_all(&after_1, "");

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

        // Try to reconnect to cache db and insert or update
        if db_available && etym_mode {
            if let Ok(db_conn) = Connection::open(&db_path) {
                if force_fetch && cache_hit {
                    let _ = db_conn.execute(
                        "UPDATE etymology SET content = (?1) WHERE word = (?2)",
                        [&final_output, &desired_word],
                    );
                } else {
                    let _ = db_conn.execute(
                        "INSERT INTO etymology (word, content) VALUES (?1, ?2)",
                        [&desired_word, &final_output],
                    );
                }
            }
        } else if db_available {
            if let Ok(db_conn) = Connection::open(&db_path) {
                if force_fetch && cache_hit {
                    let _ = db_conn.execute(
                        "UPDATE dictionary SET content = (?1) WHERE word = (?2)",
                        [&final_output, &desired_word],
                    );
                } else {
                    let _ = db_conn.execute(
                        "INSERT INTO dictionary (word, content) VALUES (?1, ?2)",
                        [&desired_word, &final_output],
                    );
                }
            }
        }

        // We still need to print results, of course
        // Also clear the spinner
        pb.finish_and_clear();
        print!("{}", final_output);
    } else {
        // If we didn't get an etymology result, stop here
        if etym_mode {
            pb.finish_and_clear();
            println!("Etymology not found");
            return Ok(());
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

            let pandoc = Command::new("pandoc")
                .arg(pandoc_input.path())
                .arg("-f")
                .arg("html+smart-native_divs")
                .arg("-t")
                .arg("plain")
                .output()
                .expect("Failed to execute Pandoc");

            let pandoc_output =
                str::from_utf8(&pandoc.stdout).expect("Failed to convert Pandoc output to string");

            // Print an explanatory message, then the results
            // Also clear the spinner
            pb.finish_and_clear();
            println!("Did you mean:\n");
            print!("{}", pandoc_output);
        } else {
            // If still no dice...
            pb.finish_and_clear();
            println!("Definition not found");
        }
    }

    Ok(())
}
