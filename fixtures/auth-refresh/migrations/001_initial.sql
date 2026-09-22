-- Protected fixture data. Fixing refresh logic must not modify migrations.
CREATE TABLE synthetic_sessions (id INTEGER PRIMARY KEY, generation INTEGER);
