#![warn(clippy::pedantic, clippy::nursery, clippy::cargo)]

mod cache;

use anyhow::anyhow;
use cache::Cache;
use clap::{Arg, ArgAction, command};
use directories::ProjectDirs;
use gloss_word::{
    LookupMode, compile_results, get_response_text, get_sections, pandoc_fallback, pandoc_primary,
    take_chunk,
};
use indicatif::{ProgressBar, ProgressStyle};
use scraper::{ElementRef, Selector};

#[allow(clippy::too_many_lines)]
fn main() -> Result<(), anyhow::Error> {
    //
    // CLI SETUP
    //

    let matches = command!()
        .arg(
            Arg::new("clear-cache")
                .long("clear-cache")
                .help("Delete cache directory and its contents")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("etymology")
                .short('e')
                .long("etymology")
                .help("Search for etymology instead of definition")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("fetch-update")
                .short('f')
                .long("fetch-update")
                .help("Fetch new data; update cache if applicable")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("INPUT")
                .help("The word or phrase to look up")
                .required_unless_present("clear-cache"),
        )
        .get_matches();

    //
    // "GLOBAL" VARIABLES
    //

    // Do we have flags?
    let clear_cache = matches.get_flag("clear-cache");
    let lookup_mode = if matches.get_flag("etymology") {
        LookupMode::Etymology
    } else {
        LookupMode::Definition
    };
    let force_fetch = matches.get_flag("fetch-update");

    // Take input and lowercase it
    let desired_word = if clear_cache {
        String::new() // Placeholder; we'll return soon, anyway
    } else {
        let input_word: &String = matches.get_one("INPUT").unwrap(); // Should be ok
        input_word.to_lowercase()
    };

    let mut cache = None;

    //
    // CACHE DIRECTORY
    //

    // Most operations here can fail silently; caching is optional
    if let Some(proj_dirs) = ProjectDirs::from("com", "theobeers", "gloss-word") {
        let cache_dir = proj_dirs.cache_dir();

        // If we have clear-cache flag, handle it and return
        if clear_cache {
            if !cache_dir.exists() {
                return Err(anyhow!("Cache directory not found"));
            }

            trash::delete(cache_dir)?;
            eprintln!("Cache directory deleted");
            return Ok(());
        }

        // If we don't have the cache dir yet, try to create it
        if !cache_dir.exists() {
            let _dir = std::fs::create_dir_all(cache_dir);
        }

        cache = Cache::open(&cache_dir.join("entries.sqlite")).ok();
    }

    //
    // DB SETUP & CHECK FOR CACHED RESULTS
    //

    // Again, these operations can fail silently
    if let Some(cache) = &cache {
        // If we got a cache hit, handle it (usually print and return)
        if let Ok(entry) = cache.get(&desired_word, lookup_mode)
            && !force_fetch
        {
            print!("{entry}");
            return Ok(());
        }
    }

    // Moving on...

    //
    // SCRAPING & CACHING
    //

    // Start a progress spinner; this could take a second
    let pb = ProgressBar::new_spinner();
    pb.enable_steady_tick(core::time::Duration::from_millis(80));
    pb.set_style(
        ProgressStyle::default_spinner()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
            .template("{spinner} {msg}")
            .unwrap(),
    );
    pb.set_message("Fetching...");

    // Build the relevant URL
    let lookup_url = lookup_mode.lookup_url(&desired_word);

    // Make HTTP request and read response body into string
    let response_text = get_response_text(&lookup_url)?;

    // Take desired chunk of response text (in definition mode)
    // In any case, parse what we have as an HTML tree
    let parsed_chunk = take_chunk(&response_text);

    // Take specific selectors that we want
    let sections = get_sections(lookup_mode, &parsed_chunk);

    // If we got one or more sections...
    if !sections.is_empty() {
        // Compile results into string
        let results = compile_results(lookup_mode, sections);

        // Call out to Pandoc
        let final_output = pandoc_primary(&results, lookup_mode)?;

        // Try to cache result; this can fail silently
        if let Some(cache) = &cache {
            let _update = cache.put(&desired_word, lookup_mode, &final_output);
        }

        // We still need to print results, of course (after clearing the spinner)
        pb.finish_and_clear();
        print!("{final_output}");
        return Ok(());
    }

    //
    // FALLBACK
    //

    // If we failed to get an etymology result, stop here
    if lookup_mode == LookupMode::Etymology {
        pb.finish_and_clear();
        return Err(anyhow!("Etymology not found"));
    }

    // In dictionary mode, we can check for a list of similar words
    let suggestions_selector = Selector::parse("ul.suggestions li").unwrap();
    let suggestions_vec: Vec<ElementRef> = parsed_chunk.select(&suggestions_selector).collect();

    // If we got something...
    if !suggestions_vec.is_empty() {
        let mut results = String::new();

        for element in &suggestions_vec {
            results.push_str(&element.html());
        }

        // Call out to Pandoc
        let pandoc_output = pandoc_fallback(&results)?;

        // Print an explanatory message, then the results (after clearing the spinner)
        pb.finish_and_clear();
        println!("Did you mean:\n");
        print!("{pandoc_output}");
        return Ok(());
    }

    // If still no dice...
    pb.finish_and_clear();
    Err(anyhow!("Definition not found"))
}
