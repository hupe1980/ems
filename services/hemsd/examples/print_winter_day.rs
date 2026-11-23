//! Print the winter report used in the simulation guide.
fn main() {
    let scenario = hemsd::Scenario::winter_with_grid_event(hemsd::HouseholdConfig::default());
    let day = hemsd::run(&scenario).expect("the winter reference day");
    print!("{}", hemsd::render::day(&scenario, &day));
}
