fn main() {
    let result = dep_repro::test_it();
    println!("Hello from dep-repro: {:?}", result.data);
}
