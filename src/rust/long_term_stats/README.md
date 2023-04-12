# Long Term Stats

We'd really rather you let us host your long-term statistics. It's a lot
of work, and gives us a revenue stream to keep building LibreQoS.

If you really want to self-host, setup is a bit convoluted - but we won't
stop you.

## PostgreSQL

* Install PostgreSQL somewhere on your network. You only want one PostgreSQL host per long-term node stats cluster.
* Setup the database schema (TBD).
* Put the connection string for your database in `/etc/lqdb` on each host.

