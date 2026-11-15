#!/usr/bin/env bash
# Starts a throwaway MariaDB on 127.0.0.1:3306 with root password "root" (docker, host network so that
# the generated 'kora_x'@'127.0.0.1' accounts can log in), and makes sure a client is installed.
set -euo pipefail
NAME=kora-smoke-db
docker rm -f "$NAME" >/dev/null 2>&1 || true
docker run -d --name "$NAME" --network host -e MARIADB_ROOT_PASSWORD=root mariadb:11 >/dev/null
for _ in $(seq 1 60); do
  if docker exec -e MYSQL_PWD=root "$NAME" mariadb -h127.0.0.1 -uroot -e 'SELECT 1' >/dev/null 2>&1; then echo "MariaDB is up"; break; fi
  sleep 2
done
docker exec -e MYSQL_PWD=root "$NAME" mariadb -h127.0.0.1 -uroot -e 'SELECT VERSION()'
# kora talks to the server with a mariadb/mysql command line client on the host.
if ! command -v mariadb >/dev/null 2>&1; then
  sudo apt-get update -qq && sudo apt-get install -y -qq mariadb-client || true
fi
command -v mariadb || command -v mysql
