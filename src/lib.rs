pub mod attentive_waiter;
pub mod once_waiter;
pub mod spinlock_waiter;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
