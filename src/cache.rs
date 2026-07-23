use gloss_word::LookupMode;
use rusqlite::Connection;
use std::path::Path;

pub struct Cache {
    connection: Connection,
}

impl Cache {
    pub fn open(path: &Path) -> Result<Self, rusqlite::Error> {
        let connection = Connection::open(path)?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS dictionary (
                word        TEXT UNIQUE NOT NULL,
                content     TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS etymology (
                word        TEXT UNIQUE NOT NULL,
                content     TEXT NOT NULL
            );",
        )?;
        Ok(Self { connection })
    }

    pub fn get(&self, word: &str, mode: LookupMode) -> Result<String, rusqlite::Error> {
        let query = format!("SELECT content FROM {} WHERE word = ?1", table_name(mode));
        let mut statement = self.connection.prepare(&query)?;
        statement.query_row([word], |row| row.get(0))
    }

    pub fn put(&self, word: &str, mode: LookupMode, content: &str) -> Result<(), rusqlite::Error> {
        let query = format!(
            "INSERT INTO {} (word, content) VALUES (?1, ?2)
             ON CONFLICT(word) DO UPDATE SET content = excluded.content",
            table_name(mode)
        );
        self.connection.execute(&query, [word, content])?;
        Ok(())
    }
}

const fn table_name(mode: LookupMode) -> &'static str {
    match mode {
        LookupMode::Definition => "dictionary",
        LookupMode::Etymology => "etymology",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cache() -> Cache {
        Cache::open(Path::new(":memory:")).unwrap()
    }

    #[test]
    fn round_trip_handles_apostrophes() {
        let cache = test_cache();

        cache
            .put("o'clock", LookupMode::Definition, "a time")
            .unwrap();

        assert_eq!(
            cache.get("o'clock", LookupMode::Definition).unwrap(),
            "a time"
        );
    }

    #[test]
    fn update_replaces_existing_content_in_the_selected_table() {
        let cache = test_cache();

        cache.put("forest", LookupMode::Etymology, "old").unwrap();
        cache.put("forest", LookupMode::Etymology, "new").unwrap();

        assert_eq!(cache.get("forest", LookupMode::Etymology).unwrap(), "new");
        assert!(cache.get("forest", LookupMode::Definition).is_err());
    }
}
