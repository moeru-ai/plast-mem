set dotenv-load

# show available recipes.
list:
  @just --list

# running plast-mem.
run *args:
  bacon run -- {{args}}

# building production.
build:
  cargo build --release

# format code. (args example: just fmt --check)
fmt *args='':
  cargo fmt --all {{args}}

# check code. (args example: just check --quiet)
check *args='':
  cargo check --all {{args}}

# lint code. (args example: just lint --fix)
lint *args='':
  cargo clippy {{args}} -- -W clippy::pedantic -W clippy::nursery -A clippy::missing-errors-doc

# running tests.
test *args='':
  cargo test --all {{args}}

# update dependencies.
up:
  cargo update

# running database. (args example: just docker down)
docker *args='up -d':
  docker compose {{args}}

# running plast-mem with database. (args example: just docker-prod down)
docker-prod *args='up -d':
  docker compose -f docker-compose.yml -f docker-compose.prod.yml {{args}}

# use db_* without underscores.
db *args='migration up':
  just db_{{args}}

# apply migrations to database.
db_migration *args='up':
  sea-orm-cli migrate {{args}} -d crates/migration -u $DATABASE_URL

# generate entities from database.
db_generate: (db_migration 'up')
  sea-orm-cli generate entity -l -o crates/entities/src -u $DATABASE_URL --ignore-tables seaql_migrations,spatial_ref_sys
