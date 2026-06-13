fn main(payment_amount: u64, spending_limit: u64) {
    assert(payment_amount > 0);
    assert(payment_amount <= spending_limit);
}

