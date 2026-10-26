//! Public ids for posts and files: 8 random characters from a 31-character alphabet
//! (lowercase and digits without the look-alikes `0 o 1 l i`). Random rather than sequential
//! so ids reveal nothing about how many private posts exist or when they were written.

use rand::RngExt;

const ALPHABET: &[u8] = b"23456789abcdefghjkmnpqrstuvwxyz";
const LEN: usize = 8;

pub fn new_id() -> String {
    let mut rng = rand::rng();
    (0..LEN).map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char).collect()
}

/// Cheap shape check so obviously bogus ids never reach the database.
pub fn is_valid(id: &str) -> bool {
    id.len() == LEN && id.bytes().all(|b| ALPHABET.contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_use_the_alphabet() {
        assert_eq!(ALPHABET.len(), 31);
        for _ in 0..1000 {
            let id = new_id();
            assert!(is_valid(&id), "{id}");
        }
        assert!(!is_valid("k7mq3xz"));
        assert!(!is_valid("k7mq3xzO"));
        assert!(!is_valid("k7mq3xz1"));
    }
}
