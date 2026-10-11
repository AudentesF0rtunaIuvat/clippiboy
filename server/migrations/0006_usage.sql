-- Anonymous usage statistics. `day` is the UTC date at the server. Nothing here
-- names a person: an install is a random id the app made up, a download is a
-- count per link and day. Rows older than 400 days are swept by the cron.

-- Downloads through clippiboy.com/get/<ref>.
CREATE TABLE downloads (
  day TEXT NOT NULL,
  ref TEXT NOT NULL,
  count INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (day, ref)
);

-- One row per install; source and heard keep the first value they got.
CREATE TABLE installs (
  id TEXT PRIMARY KEY,
  source TEXT,
  heard TEXT,
  first_day TEXT NOT NULL,
  last_day TEXT NOT NULL,
  version TEXT NOT NULL
);
CREATE INDEX installs_source ON installs(source);

-- One row per install and day it was used.
CREATE TABLE install_days (
  id TEXT NOT NULL,
  day TEXT NOT NULL,
  clips INTEGER NOT NULL DEFAULT 0,
  shots INTEGER NOT NULL DEFAULT 0,
  recordings INTEGER NOT NULL DEFAULT 0,
  features INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (id, day)
);
CREATE INDEX install_days_day ON install_days(day);
