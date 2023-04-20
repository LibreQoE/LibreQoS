# Long Term Stats

We'd really rather you let us host your long-term statistics. It's a lot
of work, and gives us a revenue stream to keep building LibreQoS.

If you really want to self-host, setup is a bit convoluted - but we won't
stop you.

## PostgreSQL

* Install PostgreSQL somewhere on your network. You only want one PostgreSQL host per long-term node stats cluster.
* Setup the database schema (TBD).
* Put the connection string for your database in `/etc/lqdb` on each host.
* Install the `sqlx` tool with `cargo install sqlx-cli --no-default-features --features rustls,postgres`

## For each stats node in the cluster

* Install InfluxDB.
* Install lts_node.
* Setup `/etc/lqdb`.
* Copy `lts_keys.bin` from the license server to the `lts_node` directory.
* Run the process.
* Login to the licensing server, and run `licman host add <ip of the new host>`
