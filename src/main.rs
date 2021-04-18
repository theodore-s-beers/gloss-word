use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::{fs, str};

use anyhow::{anyhow, Context};
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

fn main() -> Result<(), anyhow::Error> {
    //
    // CLI SETUP
    //

    let matches = App::new("gloss-word")
        .version(crate_version!())
        .author("Theo Beers <theo.beers@fu-berlin.de>")
        .about("A simple English dictionary lookup utility")
        .arg(
            Arg::with_name("clear-cache")
                .long("clear-cache")
                .help("Delete cache directory and its contents"),
        )
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
                .required_unless("clear-cache")
                .index(1),
        )
        .get_matches();

    //
    // GLOBAL VARIABLES
    //

    // Do we have flags?
    let clear_cache = matches.is_present("clear-cache");
    let etym_mode = matches.is_present("etymology");
    let force_fetch = matches.is_present("fetch-update");

    // Take input and lowercase it
    // Is this ok to unwrap?
    let mut desired_word = String::new();
    if !clear_cache {
        desired_word = matches.value_of("INPUT").unwrap().to_lowercase();
    }

    // What should be the path to the cache db? Is the db accessible?
    let mut db_path = PathBuf::new();
    let mut db_available = false;

    // Did we get a cache hit?
    let mut cache_hit = false;

    //
    // CACHE DIRECTORY
    //

    if let Some(proj_dirs) = ProjectDirs::from("com", "theobeers", "gloss-word") {
        let cache_dir = proj_dirs.cache_dir();

        if clear_cache && cache_dir.exists() {
            trash::delete(cache_dir)?;
            eprintln!("Cache directory deleted");
            return Ok(());
        } else if clear_cache {
            return Err(anyhow!("Cache directory not found"));
        }

        // If we don't have the cache dir yet, try to create it
        if !cache_dir.exists() {
            let _ = fs::create_dir_all(cache_dir);
        }

        // Construct appropriate path for db
        db_path.push(cache_dir);
        db_path.push("entries.sqlite");
    }

    //
    // DB SETUP & CHECK FOR CACHED RESULTS
    //

    if let Ok(db_conn) = Connection::open(&db_path) {
        db_available = true;

        let _ = db_conn.execute(
            "CREATE TABLE IF NOT EXISTS dictionary (
                    word        TEXT UNIQUE NOT NULL,
                    content     TEXT NOT NULL
                )",
            [],
        );

        let _ = db_conn.execute(
            "CREATE TABLE IF NOT EXISTS etymology (
                    word        TEXT UNIQUE NOT NULL,
                    content     TEXT NOT NULL
                )",
            [],
        );

        if let Ok(entry) = query_db(db_conn, &desired_word, etym_mode) {
            if force_fetch {
                cache_hit = true
            } else {
                print!("{}", entry);
                return Ok(());
            }
        }
    }

    // Moving on...

    //
    // SCRAPING & CACHING
    //

    // Start a progress spinner; this could take a second
    let pb = ProgressBar::new_spinner();
    pb.enable_steady_tick(80);
    pb.set_style(
        ProgressStyle::default_spinner()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
            .template("{spinner} {msg}"),
    );
    pb.set_message("Fetching...");

    // Build the relevant URL
    let mut lookup_url: String;

    if etym_mode {
        lookup_url = "https://www.etymonline.com/word/".to_string();
        lookup_url += &desired_word.replace(" ", "%20");
    } else {
        lookup_url = "https://www.thefreedictionary.com/".to_string();
        lookup_url += &desired_word.replace(" ", "+");
    }

    // Make request and read response body into string
    let response_text = isahc::get(lookup_url)
        .context("Failed to complete HTTP request")?
        .text()
        .context("Failed to read HTTP response body to string")?;

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
            // If definition, set up a few more selectors for desired elements
            let element_selectors = Selector::parse("div.pseg, h2, hr.hmsep").unwrap();

            // Push selected elements from first/only section
            for element in section_vec[0].select(&element_selectors) {
                results.push_str(&element.html());
            }
        }

        // Call out to Pandoc
        let final_output = pandoc_primary(etym_mode, results)?;

        // Try to cache result
        if db_available {
            let _ = update_cache(
                cache_hit,
                db_path,
                &desired_word,
                etym_mode,
                &final_output,
                force_fetch,
            );
        }

        // We still need to print results, of course
        // Also clear the spinner
        pb.finish_and_clear();
        print!("{}", final_output);
        return Ok(());
    }

    // Moving on...

    //
    // FALLBACK
    //

    // If we failed to get an etymology result, stop here
    if etym_mode {
        pb.finish_and_clear();
        return Err(anyhow!("Etymology not found"));
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

        // Call out to Pandoc
        let pandoc_output = pandoc_fallback(results)?;

        // Print an explanatory message, then the results
        // Also clear the spinner
        pb.finish_and_clear();
        println!("Did you mean:\n");
        print!("{}", pandoc_output);
        return Ok(());
    }

    // If still no dice...
    pb.finish_and_clear();
    Err(anyhow!("Definition not found"))
}

fn pandoc_fallback(results: String) -> Result<String, anyhow::Error> {
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

    let pandoc_output =
        str::from_utf8(&pandoc.stdout).context("Failed to convert Pandoc output to string")?;

    Ok(pandoc_output.to_string())
}

fn pandoc_primary(etym_mode: bool, results: String) -> Result<String, anyhow::Error> {
    let mut final_output = String::new();

    let mut pandoc_input = NamedTempFile::new().context("Failed to create tempfile")?;
    write!(pandoc_input, "{}", results).context("Failed to write to tempfile")?;

    let pandoc_1 = Command::new("pandoc")
        .arg(pandoc_input.path())
        .arg("-f")
        .arg("html+smart-native_divs")
        .arg("-t")
        .arg("markdown")
        .arg("--wrap=none")
        .output()
        .context("Failed to execute Pandoc")?;

    let output_1 =
        str::from_utf8(&pandoc_1.stdout).context("Failed to convert Pandoc output to string")?;

    if etym_mode {
        let re_quotes = Regex::new(r#"\\""#).unwrap();
        let after_1 = re_quotes.replace_all(&output_1, r#"""#).to_string();

        let re_figures = Regex::new(r#"(?m)\n\n!\[.+$"#).unwrap();
        let after_2 = re_figures.replace_all(&after_1, "");

        let mut input_file_2 = NamedTempFile::new().context("Failed to create tempfile")?;
        write!(input_file_2, "{}", after_2).context("Failed to write to tempfile")?;

        let pandoc_2 = Command::new("pandoc")
            .arg(input_file_2.path())
            .arg("-t")
            .arg("plain")
            .output()
            .context("Failed to execute Pandoc")?;

        let output_2 = str::from_utf8(&pandoc_2.stdout)
            .context("Failed to convert Pandoc output to string")?;

        final_output.push_str(output_2);
    } else {
        let re_list_1 = Regex::new(r"\n\*\*(?P<a>\d+\.)\*\*").unwrap();
        let after_1 = re_list_1.replace_all(&output_1, "\n$a").to_string();

        let re_list_2 = Regex::new(r"\n\*\*(?P<b>[a-z]\.)\*\*").unwrap();
        let after_2 = re_list_2.replace_all(&after_1, "\n    $b").to_string();

        let mut input_file_2 = NamedTempFile::new().context("Failed to create tempfile")?;
        write!(input_file_2, "{}", after_2).context("Failed to write to tempfile")?;

        let pandoc_2 = Command::new("pandoc")
            .arg(input_file_2.path())
            .arg("-t")
            .arg("plain")
            .output()
            .context("Failed to execute Pandoc")?;

        let output_2 = str::from_utf8(&pandoc_2.stdout)
            .context("Failed to convert Pandoc output to string")?;

        final_output.push_str(output_2);
    }

    Ok(final_output)
}

fn query_db(
    db_conn: Connection,
    desired_word: &str,
    etym_mode: bool,
) -> Result<String, rusqlite::Error> {
    let mut query = String::new();

    if etym_mode {
        query.push_str("SELECT * FROM etymology WHERE word = '");
    } else {
        query.push_str("SELECT * FROM dictionary WHERE word = '");
    }

    query.push_str(desired_word);
    query.push('\'');

    let mut stmt = db_conn.prepare(&query)?;

    let entry = stmt.query_row([], |row| {
        Ok(Entry {
            word: row.get(0)?,
            content: row.get(1)?,
        })
    })?;

    Ok(entry.content)
}

fn update_cache(
    cache_hit: bool,
    db_path: PathBuf,
    desired_word: &str,
    etym_mode: bool,
    final_output: &str,
    force_fetch: bool,
) -> Result<(), rusqlite::Error> {
    let db_conn = Connection::open(&db_path)?;

    if force_fetch && cache_hit {
        if etym_mode {
            db_conn.execute(
                "UPDATE etymology SET content = (?1) WHERE word = (?2)",
                [final_output, desired_word],
            )?;
        } else {
            db_conn.execute(
                "UPDATE dictionary SET content = (?1) WHERE word = (?2)",
                [final_output, desired_word],
            )?;
        }
    } else if etym_mode {
        db_conn.execute(
            "INSERT INTO etymology (word, content) VALUES (?1, ?2)",
            [desired_word, final_output],
        )?;
    } else {
        db_conn.execute(
            "INSERT INTO dictionary (word, content) VALUES (?1, ?2)",
            [desired_word, final_output],
        )?;
    }

    Ok(())
}
