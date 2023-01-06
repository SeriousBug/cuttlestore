## Running these benchmarks

You will need `docker-compose` (or otherwise some way to run redis at the default port).
While inside the `benches` folder, run `docker compose up` to start redis.

Make sure you have [Criterion.rs](https://github.com/bheisler/criterion.rs) installed.
In a separate terminal, open the root of the project and run `cargo criterion`.
Running all the benchmarks may take a few minutes.
