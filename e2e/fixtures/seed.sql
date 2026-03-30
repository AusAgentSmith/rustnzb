-- Test groups (2 subscribed, 1 not)
INSERT INTO groups (id, name, description, subscribed, article_count, first_article, last_article, last_scanned)
VALUES
  (1, 'alt.test', 'General testing', 1, 100, 1, 100, 50),
  (2, 'alt.binaries.test', 'Binary testing', 1, 500, 1, 500, 500),
  (3, 'misc.test', 'Misc group', 0, 30, 1, 30, 0);

-- Headers for group 1
INSERT INTO headers (id, group_id, article_num, subject, author, date, message_id, references_, bytes, lines)
VALUES
  (1, 1, 1, 'Test Post Alpha', 'alice@test.com', '2026-03-01 10:00:00', 'msg1@test', '', 512, 10),
  (2, 1, 2, 'Re: Test Post Alpha', 'bob@test.com', '2026-03-01 11:00:00', 'msg2@test', 'msg1@test', 256, 5),
  (3, 1, 3, 'Binary File [1/3]', 'poster@news', '2026-03-02 09:00:00', 'bin1@test', '', 2048000, 5000),
  (4, 1, 4, 'Binary File [2/3]', 'poster@news', '2026-03-02 09:01:00', 'bin2@test', '', 2048000, 5000),
  (5, 1, 5, 'Binary File [3/3]', 'poster@news', '2026-03-02 09:02:00', 'bin3@test', '', 2048000, 5000);
