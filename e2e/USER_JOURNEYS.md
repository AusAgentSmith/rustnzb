# rustnzb — User Journey Specs

Each journey maps to one or more Playwright spec files.
**Preconditions** describe the state the test harness must set up before the scenario runs.
**Assertions** are the expected observable outcomes — these become `expect()` calls.

---

## 1. First Boot & Account Setup

### 1.1 First-run redirects to login in setup mode
**Precondition:** Fresh DB, no credentials, no servers.
**Steps:**
1. Navigate to `/`
**Assertions:**
- Redirected to `/login`
- "Create your account to get started" subtitle visible
- "Create Account" button visible (not "Sign In")
- Confirm Password field visible

### 1.2 Account creation with mismatched passwords shows error
**Precondition:** Same as 1.1.
**Steps:**
1. Navigate to `/login`
2. Fill username, fill password, fill confirmPassword with different value
3. Click "Create Account"
**Assertions:**
- Error message "Passwords do not match" visible
- Still on `/login`

### 1.3 Account creation succeeds and lands on welcome screen
**Precondition:** Same as 1.1.
**Steps:**
1. Navigate to `/login`
2. Fill valid username and matching passwords
3. Click "Create Account"
**Assertions:**
- Redirected to `/welcome`
- "Welcome to rustnzb" heading visible
- "Import from SABnzbd" button visible
- "Set up manually" button visible
- "Skip for now" link visible

### 1.4 Welcome screen — skip goes to queue
**Precondition:** Logged in, no servers configured.
**Steps:**
1. Navigate to `/welcome`
2. Click "Skip for now →"
**Assertions:**
- Redirected to `/queue`
- Queue empty state visible

### 1.5 Welcome screen — "Set up manually" goes to settings
**Precondition:** Logged in, no servers configured.
**Steps:**
1. Navigate to `/welcome`
2. Click "Set up manually"
**Assertions:**
- Redirected to `/settings`
- News servers tab visible

### 1.6 Welcome screen skipped if servers already configured
**Precondition:** Logged in, at least one server in config.
**Steps:**
1. Navigate to `/welcome`
**Assertions:**
- Immediately redirected to `/queue`

### 1.7 SABnzbd live import — validation errors
**Precondition:** Logged in, no servers.
**Steps:**
1. Navigate to `/welcome`
2. Click "Import from SABnzbd"
3. Click "Fetch config" without filling in any fields
**Assertions:**
- Error "SABnzbd URL is required" visible
- Still on connect step

### 1.8 SABnzbd live import — unreachable host shows error
**Precondition:** Logged in, no servers.
**Steps:**
1. Navigate to `/welcome`
2. Click "Import from SABnzbd"
3. Enter `http://localhost:1` as URL, any string as API key
4. Click "Fetch config"
**Assertions:**
- Error message containing "Failed to connect" visible
- Still on connect step

### 1.9 SABnzbd INI upload — shows preview
**Precondition:** Logged in, no servers. A valid `sabnzbd.ini` fixture available.
**Steps:**
1. Navigate to `/welcome`
2. Click "Import from SABnzbd"
3. Switch to "Config file (.ini)" tab
4. Upload the fixture ini file
5. Click "Fetch config"
**Assertions:**
- Preview step shown
- At least one server card visible with host/port info
- "Apply & continue" button visible

### 1.10 SABnzbd INI import with masked passwords — apply blocked
**Precondition:** Preview step shown with at least one server that has `password_masked = true`.
**Steps:**
1. Do not fill in any password fields
2. Click "Apply & continue"
**Assertions:**
- Button is disabled
- Hint "Enter passwords for all masked servers" visible

### 1.11 SABnzbd import apply succeeds → queue
**Precondition:** Preview step shown with no masked passwords (either plain INI or passwords entered).
**Steps:**
1. Click "Apply & continue"
**Assertions:**
- Applying spinner shown briefly
- Redirected to `/queue`
- Server now appears in `/settings` → News servers

### 1.12 Subsequent login (not setup) goes to queue directly
**Precondition:** Account already exists (credentials in DB), at least one server.
**Steps:**
1. Navigate to `/login`
2. Fill valid credentials
3. Click "Sign In"
**Assertions:**
- Redirected to `/queue` (not `/welcome`)
- "Sign in to continue" subtitle was shown (not setup mode)

---

## 2. News Server Management

### 2.1 Add a news server
**Precondition:** Logged in, no servers.
**Steps:**
1. Navigate to `/settings`
2. Click "News servers" tab (default)
3. Click "+ Add server"
4. Fill host, port, connections; toggle SSL
5. Click "Save"
**Assertions:**
- Server row appears in list with host name
- Success snackbar "Server added" visible

### 2.2 Server form validation — empty host
**Precondition:** Settings page, server form open.
**Steps:**
1. Click "+ Add server"
2. Leave host blank
3. Click "Save"
**Assertions:**
- (browser validation or error) prevents save / error shown

### 2.3 Edit existing server
**Precondition:** One server exists.
**Steps:**
1. Navigate to `/settings`
2. Click edit icon on the existing server
3. Change connections count
4. Click "Save"
**Assertions:**
- Updated connections count visible in server row
- Snackbar "Server updated"

### 2.4 Delete server with confirmation
**Precondition:** One server exists.
**Steps:**
1. Navigate to `/settings`
2. Click delete icon on server
3. Confirm the dialog
**Assertions:**
- Server row removed from list
- Snackbar "Server removed"

### 2.5 Test server connection — success
**Precondition:** A reachable test NNTP server configured (or mock).
**Steps:**
1. Navigate to `/settings`
2. Click "Test" on the server
**Assertions:**
- Snackbar with "Successfully connected to" message

### 2.6 Test server connection — failure
**Precondition:** A server pointing to `localhost:1` configured.
**Steps:**
1. Navigate to `/settings`
2. Click "Test"
**Assertions:**
- Snackbar with "Connection failed" or "timed out" message

---

## 3. Category Management

### 3.1 Add a category
**Precondition:** Logged in.
**Steps:**
1. Navigate to `/settings` → Categories tab
2. Click "+ Add category"
3. Fill name (e.g. "movies"), optionally fill output dir
4. Save
**Assertions:**
- Category row appears
- Snackbar "Category added"

### 3.2 Edit category output directory
**Precondition:** Category "movies" exists.
**Steps:**
1. Click edit on "movies"
2. Change output_dir
3. Save
**Assertions:**
- Updated dir shown in row
- Snackbar "Category updated"

### 3.3 Delete category
**Precondition:** Category exists.
**Steps:**
1. Click delete on category
2. Confirm
**Assertions:**
- Category row removed
- Snackbar "Category removed"

---

## 4. Queue — NZB Download

### 4.1 Add NZB via file upload — appears in queue
**Precondition:** Server configured, mock NNTP available.
**Steps:**
1. Navigate to `/queue`
2. Click "Add NZB" (or drag-drop area)
3. Upload a fixture `.nzb` file
**Assertions:**
- Job row appears in queue
- Job name matches NZB filename
- Status shows "Queued" or "Downloading"
- Size and article count visible

### 4.2 Add NZB via URL
**Precondition:** Server configured, NZB URL accessible.
**Steps:**
1. Navigate to `/queue`
2. Open "Add NZB" → "From URL" tab
3. Paste URL
4. Confirm
**Assertions:**
- Job appears in queue

### 4.3 Pause individual job
**Precondition:** At least one active download.
**Steps:**
1. Click pause icon on a job
**Assertions:**
- Job status changes to "Paused"
- Pause button becomes resume button

### 4.4 Resume paused job
**Precondition:** One job is paused.
**Steps:**
1. Click resume icon on the paused job
**Assertions:**
- Status changes back to "Downloading" or "Queued"

### 4.5 Pause all / Resume all
**Precondition:** Multiple active downloads.
**Steps:**
1. Click global pause button in header
**Assertions:**
- All jobs show Paused status
- Speed indicator drops to 0
2. Click resume button
**Assertions:**
- Downloads resume

### 4.6 Delete job from queue
**Precondition:** Job in queue.
**Steps:**
1. Click delete on a job
2. Confirm if prompted
**Assertions:**
- Job disappears from queue

### 4.7 Bulk select and delete
**Precondition:** Multiple jobs in queue.
**Steps:**
1. Check checkboxes on two jobs
2. Click "Delete selected" (or bulk delete button)
3. Confirm
**Assertions:**
- Both jobs removed

### 4.8 Bulk priority change
**Precondition:** Multiple jobs in queue.
**Steps:**
1. Select two jobs
2. Set priority to "High"
**Assertions:**
- Both jobs show High priority indicator

### 4.9 Change job category
**Precondition:** Job in queue, "movies" category exists.
**Steps:**
1. Open job's category dropdown
2. Select "movies"
**Assertions:**
- Job shows "movies" category

### 4.10 Move job position (priority reorder)
**Precondition:** Multiple queued jobs.
**Steps:**
1. Move first job to position 3 (via drag or UI control)
**Assertions:**
- Job appears at new position in list

### 4.11 Speed limit shows in status bar
**Precondition:** Speed limit set to 5 MB/s in settings.
**Steps:**
1. Navigate to `/queue`
**Assertions:**
- Status bar shows speed limit value

### 4.12 Empty queue shows empty state
**Precondition:** No jobs.
**Steps:**
1. Navigate to `/queue`
**Assertions:**
- "No downloads in queue" (or equivalent) message visible
- No job rows present

---

## 5. Download History

### 5.1 Completed job appears in history
**Precondition:** A job has completed (seeded into DB as completed).
**Steps:**
1. Navigate to `/history`
**Assertions:**
- Job row visible with name, status "Completed", size

### 5.2 Failed job shows error indicator
**Precondition:** A job seeded as failed.
**Steps:**
1. Navigate to `/history`
**Assertions:**
- Failed row has error styling or "Failed" status badge
- Error message accessible (expand or tooltip)

### 5.3 Retry failed job
**Precondition:** Failed job in history that has NZB data stored.
**Steps:**
1. Click retry icon on failed job
**Assertions:**
- Job reappears in `/queue`
- History entry removed or marked as retried

### 5.4 Delete history entry
**Precondition:** History has at least one entry.
**Steps:**
1. Click delete on a history entry
2. Confirm
**Assertions:**
- Entry removed from history list

### 5.5 Clear all history
**Precondition:** History has multiple entries.
**Steps:**
1. Click "Clear history" or equivalent
2. Confirm
**Assertions:**
- History list is empty

### 5.6 View job logs from history
**Precondition:** Completed job with stored logs.
**Steps:**
1. Click "Logs" or expand icon on a history entry
**Assertions:**
- Log entries rendered (timestamps, messages)

---

## 6. RSS Feeds

### 6.1 Add RSS feed
**Precondition:** Logged in.
**Steps:**
1. Navigate to `/rss`
2. Click "Add feed"
3. Fill name and URL
4. Save
**Assertions:**
- Feed row appears with name and URL
- Snackbar "Feed added"

### 6.2 Feed poll interval configurable
**Precondition:** Add feed form open.
**Steps:**
1. Set poll interval to 60 minutes
2. Save
**Assertions:**
- Feed row shows 60 min interval

### 6.3 Delete RSS feed
**Precondition:** Feed exists.
**Steps:**
1. Click delete on feed
2. Confirm
**Assertions:**
- Feed removed

### 6.4 RSS items list shows fetched items
**Precondition:** Feed seeded with items in DB.
**Steps:**
1. Navigate to `/rss`
2. Select the seeded feed
**Assertions:**
- Item rows visible with title, date
- Download button per item visible

### 6.5 Download RSS item enqueues to queue
**Precondition:** RSS item has a valid NZB URL (mock).
**Steps:**
1. Click download on an RSS item
**Assertions:**
- Success indicator
- Job appears in `/queue`

### 6.6 Add download rule
**Precondition:** Feed exists.
**Steps:**
1. Navigate to RSS rules section
2. Click "Add rule"
3. Set regex pattern (e.g. `S01E\d+`), assign to feed, set category
4. Save
**Assertions:**
- Rule row appears with pattern and feed name

### 6.7 Edit download rule
**Precondition:** Rule exists.
**Steps:**
1. Click edit on rule
2. Change regex
3. Save
**Assertions:**
- Updated regex shown in rule row

### 6.8 Delete download rule
**Precondition:** Rule exists.
**Steps:**
1. Click delete
2. Confirm
**Assertions:**
- Rule removed

---

## 7. Newsgroup Browser

### 7.1 Groups list shows subscribed groups
**Precondition:** Groups seeded (alt.test, alt.binaries.test).
**Steps:**
1. Navigate to `/groups`
**Assertions:**
- Both group names visible in left panel

### 7.2 Click group loads header list
**Precondition:** Headers seeded for alt.test.
**Steps:**
1. Click "alt.test"
**Assertions:**
- Header panel opens
- Seeded post subjects visible
- Article count shown

### 7.3 Search filters header list
**Precondition:** Headers loaded for alt.test.
**Steps:**
1. Type "Binary" in the search box, press Enter
**Assertions:**
- Only headers containing "Binary" remain
- Non-matching headers hidden

### 7.4 Select headers — download bar appears
**Precondition:** Headers visible for a group.
**Steps:**
1. Check one header checkbox
**Assertions:**
- Download bar appears at bottom
- "1 selected" count shown
- "Download Selected" button visible

### 7.5 Select all headers
**Precondition:** Headers visible.
**Steps:**
1. Click the select-all checkbox in the table header
**Assertions:**
- All header rows checked
- Count reflects total

### 7.6 Deselect removes download bar
**Precondition:** One header selected.
**Steps:**
1. Uncheck the checkbox
**Assertions:**
- Download bar disappears

### 7.7 Thread view for multi-part post
**Precondition:** Threaded headers seeded.
**Steps:**
1. Open a group with threaded data
2. Switch to thread view
**Assertions:**
- Thread root post visible with reply count
- Expanding thread shows child posts

### 7.8 Subscribe to a group
**Precondition:** Groups list has an unsubscribed group.
**Steps:**
1. Search or find an unsubscribed group
2. Click subscribe
**Assertions:**
- Group appears in subscribed list
- Snackbar or indicator confirms subscription

---

## 8. Settings — General & Paths

### 8.1 Change speed limit
**Precondition:** Logged in.
**Steps:**
1. Navigate to `/settings` → General tab
2. Set speed limit to 10 MB/s
3. Save
**Assertions:**
- Saved successfully (snackbar or no error)
- Status bar reflects new limit

### 8.2 Change complete directory
**Precondition:** Directory picker available.
**Steps:**
1. Navigate to `/settings` → Paths & disk tab
2. Click directory picker for complete dir
3. Select a valid path
4. Save
**Assertions:**
- New path shown in field
- Saved without error

### 8.3 History retention setting
**Precondition:** Settings loaded.
**Steps:**
1. Navigate to General tab
2. Set retention to 30 days
3. Save
**Assertions:**
- Value persisted (visible after page reload)

### 8.4 Max concurrent downloads
**Precondition:** Settings loaded.
**Steps:**
1. Set max active downloads to 2
2. Save
**Assertions:**
- Value persisted

---

## 9. Logs

### 9.1 Log page renders entries
**Precondition:** Backend has produced startup log entries.
**Steps:**
1. Navigate to `/logs`
**Assertions:**
- Log container visible
- At least one log entry rendered

### 9.2 Filter logs by level
**Precondition:** Log entries exist with mixed levels.
**Steps:**
1. Navigate to `/logs`
2. Select "WARN" or "ERROR" filter
**Assertions:**
- Only matching level entries shown

### 9.3 Filter logs by job ID
**Precondition:** Logs seeded with entries for a specific job ID.
**Steps:**
1. Enter the job ID in the filter field
**Assertions:**
- Only entries for that job visible

---

## 10. Authentication

### 10.1 Protected route redirects to login when unauthenticated
**Precondition:** Auth enabled, no session.
**Steps:**
1. Navigate to `/queue` without logging in
**Assertions:**
- Redirected to `/login`

### 10.2 Invalid credentials show error
**Precondition:** Account exists.
**Steps:**
1. Navigate to `/login`
2. Enter wrong password
3. Click "Sign In"
**Assertions:**
- "Invalid username or password" error visible
- Still on `/login`

### 10.3 Logout clears session
**Precondition:** Logged in.
**Steps:**
1. Click logout button
**Assertions:**
- Redirected to `/login`
- Navigating to `/queue` redirects back to `/login`

### 10.4 Session persists on page reload
**Precondition:** Logged in.
**Steps:**
1. Reload the page
**Assertions:**
- Still on authenticated view (not redirected to `/login`)

---

## 11. Navigation & Shell

### 11.1 All nav tabs present when authenticated
**Precondition:** Logged in.
**Steps:**
1. Navigate to `/`
**Assertions:**
- Queue, Groups, RSS, History, Settings, Logs links all visible

### 11.2 Active tab is highlighted
**Precondition:** Logged in.
**Steps:**
1. Navigate to `/history`
**Assertions:**
- History nav link has active styling
- Other links do not

### 11.3 Status bar shows connection state
**Precondition:** Backend healthy.
**Steps:**
1. Navigate to any page
**Assertions:**
- Status bar visible with "Connected" or speed indicator

### 11.4 Status bar shows paused state
**Precondition:** Downloads paused.
**Steps:**
1. Pause all from queue
2. Check status bar
**Assertions:**
- "Paused" indicator visible in status bar

---

## Fixture / Test Data Notes

| Fixture | Purpose |
|---------|---------|
| `fixtures/test-config.toml` | Minimal config: no auth, no servers, test port 9190 |
| `fixtures/seed.sql` | Groups + headers for newsreader tests |
| `fixtures/sabnzbd.ini` | *(to create)* Sample SABnzbd config for import tests |
| `fixtures/sample.nzb` | *(to create)* Minimal valid NZB for queue tests |
| `fixtures/sample-auth-config.toml` | *(to create)* Config with auth enabled for auth tests |

## Suggested Spec File Layout

```
e2e/tests/
  smoke.spec.ts          ← exists (basic render checks)
  first-boot.spec.ts     ← journeys 1.x
  servers.spec.ts        ← journeys 2.x
  categories.spec.ts     ← journeys 3.x
  queue.spec.ts          ← journeys 4.x
  history.spec.ts        ← journeys 5.x
  rss.spec.ts            ← journeys 6.x
  groups.spec.ts         ← journeys 7.x
  settings.spec.ts       ← journeys 8.x
  logs.spec.ts           ← journeys 9.x
  auth.spec.ts           ← journeys 10.x
  navigation.spec.ts     ← journeys 11.x
```
