pub mod boolean;
pub mod date_time;
pub mod integer;

pub fn integer(selected_text: &str, amount: i64) -> Option<String> {
    integer::increment(selected_text, amount)
}

pub fn date_time(selected_text: &str, amount: i64) -> Option<String> {
    date_time::increment(selected_text, amount)
}
