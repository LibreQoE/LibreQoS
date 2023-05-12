pub struct RedisRing {
    capacity: u32,
    connection: redis::Connection,
    key: String,
    reversed: bool,
}

impl RedisRing {
    fn get(&self) {
        self.connection.get(self.key)?
    }

    fn new(key: &str, capacity: u32, reversed: bool) -> Self {
        let client = redis::Client::open("redis://127.0.0.1/")?;
        let mut connection = client.get_connection()?;

        RedisRing {
            capacity: capacity,
            connection: connection,
            key: key.to_uppercase(),
            reversed: reversed
        }
    }

    fn clear(&self) {
        self.connection.del(self.key)?;
    }

    fn push<T>(&self, value: T) {
        if self.reversed {
            self.connection.rpush(self.key, value)?;
            self.connection.ltrim(self.key, 1, self.capacity + 1)?;
        } else {
            self.connection.lpush(self.key, value)?;
            self.connection.ltrim(self.key, 0, self.capacity)?;
        }
    }
}