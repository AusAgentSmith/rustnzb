/**
 * Mock API interceptor for rustnzb demo.
 * Intercepts XMLHttpRequest calls to /api/* and returns mock data.
 * Must be loaded BEFORE Angular boots.
 */
(function () {
  'use strict';

  // Ensure demo user appears logged in
  if (!localStorage.getItem('access_token')) {
    localStorage.setItem('access_token', 'demo-token-mock');
    localStorage.setItem('refresh_token', 'demo-refresh-mock');
  }

  // ---------- Mock Data ----------

  var logSeq = 0;
  function nextSeq() { return ++logSeq; }
  var startTime = new Date('2026-04-08T10:00:00Z').getTime();

  var MOCK = {
    '/api/auth/status': { auth_enabled: true, setup_required: false },

    '/api/status': {
      speed_bps: 77000000,
      queue_size: 5,
      queue_remaining_bytes: 6680000000,
      disk_free_bytes: 524288000000,
      paused: false,
      uptime_secs: 86400
    },

    '/api/queue': {
      jobs: [
        {
          id: '1', name: 'Ubuntu.24.04.Desktop.AMD64', category: 'linux',
          status: 'downloading', priority: 1,
          total_bytes: 4200000000, downloaded_bytes: 2730000000,
          file_count: 42, files_completed: 28,
          article_count: 5600, articles_downloaded: 3640, articles_failed: 2,
          added_at: '2026-04-08T10:00:00Z', completed_at: null,
          speed_bps: 45000000, error_message: null,
          server_stats: [
            { server_id: '1', server_name: 'Eweka', articles_downloaded: 3200, articles_failed: 1, bytes_downloaded: 2400000000 },
            { server_id: '2', server_name: 'UsenetExpress', articles_downloaded: 440, articles_failed: 1, bytes_downloaded: 330000000 }
          ]
        },
        {
          id: '2', name: 'Fedora.Workstation.41.x86_64', category: 'linux',
          status: 'downloading', priority: 1,
          total_bytes: 2100000000, downloaded_bytes: 840000000,
          file_count: 21, files_completed: 8,
          article_count: 2800, articles_downloaded: 1120, articles_failed: 0,
          added_at: '2026-04-08T10:05:00Z', completed_at: null,
          speed_bps: 32000000, error_message: null,
          server_stats: [
            { server_id: '1', server_name: 'Eweka', articles_downloaded: 1120, articles_failed: 0, bytes_downloaded: 840000000 }
          ]
        },
        {
          id: '3', name: 'Arch.Linux.2026.04', category: 'linux',
          status: 'queued', priority: 1,
          total_bytes: 950000000, downloaded_bytes: 0,
          file_count: 12, files_completed: 0,
          article_count: 1267, articles_downloaded: 0, articles_failed: 0,
          added_at: '2026-04-08T10:10:00Z', completed_at: null,
          speed_bps: 0, error_message: null,
          server_stats: []
        },
        {
          id: '4', name: 'Some.Movie.2026.1080p.BluRay.x264', category: 'movies',
          status: 'verifying', priority: 1,
          total_bytes: 8500000000, downloaded_bytes: 8500000000,
          file_count: 85, files_completed: 85,
          article_count: 11333, articles_downloaded: 11333, articles_failed: 12,
          added_at: '2026-04-08T09:30:00Z', completed_at: null,
          speed_bps: 0, error_message: null,
          server_stats: [
            { server_id: '1', server_name: 'Eweka', articles_downloaded: 10200, articles_failed: 8, bytes_downloaded: 7650000000 },
            { server_id: '2', server_name: 'UsenetExpress', articles_downloaded: 1133, articles_failed: 4, bytes_downloaded: 850000000 }
          ]
        },
        {
          id: '5', name: 'Great.TV.Show.S03E01-E04.1080p', category: 'tv',
          status: 'paused', priority: 1,
          total_bytes: 3200000000, downloaded_bytes: 1600000000,
          file_count: 32, files_completed: 16,
          article_count: 4267, articles_downloaded: 2133, articles_failed: 0,
          added_at: '2026-04-08T09:00:00Z', completed_at: null,
          speed_bps: 0, error_message: null,
          server_stats: []
        }
      ],
      total: 5,
      speed_bps: 77000000,
      paused: false
    },

    '/api/history': {
      entries: [
        {
          id: 'h1', name: 'Another.Movie.2025.2160p.WEB-DL', category: 'movies',
          status: 'completed', total_bytes: 12000000000, downloaded_bytes: 12000000000,
          added_at: '2026-04-07T18:00:00Z', completed_at: '2026-04-07T19:15:00Z',
          output_dir: '/downloads/movies/Another.Movie.2025',
          stages: [
            { name: 'Download', status: 'completed', message: null, duration_secs: 3200 },
            { name: 'Par2 Verify', status: 'completed', message: 'All files intact', duration_secs: 45 },
            { name: 'Extract', status: 'completed', message: 'Extracted 1 archive', duration_secs: 120 }
          ],
          error_message: null,
          server_stats: [
            { server_id: '1', server_name: 'Eweka', articles_downloaded: 15000, articles_failed: 3, bytes_downloaded: 11250000000 }
          ],
          has_nzb_data: true
        },
        {
          id: 'h2', name: 'Linux.Mint.22', category: 'linux',
          status: 'completed', total_bytes: 2800000000, downloaded_bytes: 2800000000,
          added_at: '2026-04-07T16:00:00Z', completed_at: '2026-04-07T16:25:00Z',
          output_dir: '/downloads/linux/Linux.Mint.22',
          stages: [
            { name: 'Download', status: 'completed', message: null, duration_secs: 900 },
            { name: 'Par2 Verify', status: 'completed', message: null, duration_secs: 20 },
            { name: 'Extract', status: 'completed', message: null, duration_secs: 60 }
          ],
          error_message: null, server_stats: [], has_nzb_data: true
        },
        {
          id: 'h3', name: 'Bad.Download.Corrupted', category: '',
          status: 'failed', total_bytes: 5000000000, downloaded_bytes: 3500000000,
          added_at: '2026-04-07T14:00:00Z', completed_at: '2026-04-07T14:45:00Z',
          output_dir: '',
          stages: [
            { name: 'Download', status: 'completed', message: null, duration_secs: 2400 },
            { name: 'Par2 Verify', status: 'failed', message: 'Missing 15 blocks, repair not possible', duration_secs: 30 }
          ],
          error_message: 'Par2 repair failed: insufficient recovery data',
          server_stats: [], has_nzb_data: false
        }
      ]
    },

    '/api/config/categories': [
      { name: 'movies', output_dir: '/downloads/movies', post_processing: 3 },
      { name: 'tv', output_dir: '/downloads/tv', post_processing: 3 },
      { name: 'linux', output_dir: '/downloads/linux', post_processing: 2 }
    ],

    '/api/config/servers': [
      {
        id: '1', name: 'Eweka', host: 'news.eweka.nl', port: 563, ssl: true,
        username: 'demo', connections: 20, priority: 0, enabled: true,
        retention_days: 5800
      },
      {
        id: '2', name: 'UsenetExpress', host: 'news.usenetexpress.com', port: 563, ssl: true,
        username: 'demo', connections: 10, priority: 1, enabled: true,
        retention_days: 3500
      }
    ],

    '/api/config/speed-limit': { speed_limit_bps: 0 },
    '/api/config/max-active-downloads': { max_active_downloads: 3 },
    '/api/config/history-retention': { retention: null },

    '/api/config/rss-feeds': [],
    '/api/rss/rules': [],
    '/api/rss/items': [],

    '/api/groups': { groups: [], total: 0 }
  };

  function generateLogs(afterSeq) {
    var levels = ['INFO', 'INFO', 'INFO', 'INFO', 'DEBUG', 'WARN'];
    var messages = [
      'Article 3641/5600 downloaded from Eweka (750 KB)',
      'Connection pool: 20/20 active on Eweka',
      'Connection pool: 8/10 active on UsenetExpress',
      'Article 1121/2800 downloaded from Eweka (750 KB)',
      'Speed: 77.0 MB/s (Eweka: 54.2 MB/s, UsenetExpress: 22.8 MB/s)',
      'File 29/42 completed: ubuntu-24.04-desktop-amd64.part029.rar',
      'Queue: 2 downloading, 1 queued, 1 verifying, 1 paused',
      'Par2 verification in progress for Some.Movie.2026.1080p.BluRay.x264',
      'Disk space: 488.3 GB free on /downloads',
      'yEnc decode: 750 KB in 0.12ms (SIMD AVX2)',
      'Article 3642/5600 downloaded from Eweka (750 KB)',
      'File assembly: writing ubuntu-24.04-desktop-amd64.part029.rar (100 MB)'
    ];
    var entries = [];
    var base = afterSeq || 0;
    var count = base === 0 ? 25 : Math.floor(Math.random() * 4) + 1;
    for (var i = 0; i < count; i++) {
      var seq = base + i + 1;
      var ts = new Date(startTime + seq * 1200).toISOString();
      entries.push({
        seq: seq,
        timestamp: ts,
        level: levels[Math.floor(Math.random() * levels.length)],
        target: 'rustnzb::download_engine',
        message: messages[Math.floor(Math.random() * messages.length)]
      });
    }
    logSeq = entries[entries.length - 1].seq;
    return { entries: entries };
  }

  function generateHistoryLogs(id) {
    var entries = [];
    var msgs = [
      { level: 'INFO', msg: 'Download started' },
      { level: 'INFO', msg: 'Connected to Eweka (20 connections)' },
      { level: 'INFO', msg: 'Downloading articles: 0/15000' },
      { level: 'INFO', msg: 'Speed: 85.2 MB/s' },
      { level: 'INFO', msg: 'Downloading articles: 7500/15000 (50%)' },
      { level: 'INFO', msg: 'Downloading articles: 15000/15000 (100%)' },
      { level: 'INFO', msg: 'Download complete, starting verification' },
      { level: 'INFO', msg: 'Par2 verify: all files intact' },
      { level: 'INFO', msg: 'Extracting archives...' },
      { level: 'INFO', msg: 'Extracted 1 archive to /downloads/movies' },
      { level: 'INFO', msg: 'Post-processing complete' }
    ];
    for (var i = 0; i < msgs.length; i++) {
      entries.push({
        seq: i + 1,
        timestamp: new Date(startTime - 86400000 + i * 300000).toISOString(),
        level: msgs[i].level,
        target: 'rustnzb',
        message: msgs[i].msg
      });
    }
    return { entries: entries };
  }

  // ---------- XHR Interceptor ----------

  var RealXHR = window.XMLHttpRequest;

  function MockXHR() {
    this._real = new RealXHR();
    this._method = '';
    this._url = '';
    this._requestHeaders = {};
    this._responseHeaders = {};
    this._intercepted = false;
    this._mockResponse = '';
    this._mockStatus = 200;

    // Copy event handler properties
    this.onreadystatechange = null;
    this.onload = null;
    this.onerror = null;
    this.onabort = null;
    this.ontimeout = null;
    this.onloadend = null;
    this.onloadstart = null;
    this.onprogress = null;
    this.upload = this._real.upload;
  }

  MockXHR.prototype.open = function (method, url) {
    this._method = method;
    this._url = url;

    // Check if this URL should be intercepted
    if (typeof url === 'string' && url.indexOf('/api/') === 0) {
      this._intercepted = true;
      var response = this._resolveResponse(method, url);
      if (response !== undefined) {
        this._mockResponse = JSON.stringify(response);
        this._mockStatus = 200;
      } else {
        // Return empty 200 for unhandled POST/PUT/DELETE
        this._mockResponse = JSON.stringify({ success: true });
        this._mockStatus = 200;
      }
    } else {
      this._real.open.apply(this._real, arguments);
    }
  };

  MockXHR.prototype._resolveResponse = function (method, url) {
    // Strip query string for matching
    var path = url.split('?')[0];
    var qs = url.indexOf('?') !== -1 ? url.split('?')[1] : '';

    // Exact match first
    if (method === 'GET' && MOCK[path] !== undefined) {
      // Special handling for logs with after_seq
      if (path === '/api/logs') {
        var afterSeq = 0;
        if (qs) {
          var match = qs.match(/after_seq=(\d+)/);
          if (match) afterSeq = parseInt(match[1], 10);
        }
        return generateLogs(afterSeq);
      }
      return MOCK[path];
    }

    // History item logs
    if (method === 'GET' && /^\/api\/history\/[^/]+\/logs$/.test(path)) {
      var id = path.split('/')[3];
      return generateHistoryLogs(id);
    }

    // For POST/PUT/DELETE, return success
    if (method !== 'GET') {
      return { success: true };
    }

    return undefined;
  };

  MockXHR.prototype.send = function () {
    if (!this._intercepted) {
      this._real.send.apply(this._real, arguments);
      return;
    }

    var self = this;

    // Simulate async response
    setTimeout(function () {
      Object.defineProperty(self, 'readyState', { value: 4, writable: true, configurable: true });
      Object.defineProperty(self, 'status', { value: self._mockStatus, writable: true, configurable: true });
      Object.defineProperty(self, 'statusText', { value: 'OK', writable: true, configurable: true });
      Object.defineProperty(self, 'responseText', { value: self._mockResponse, writable: true, configurable: true });
      Object.defineProperty(self, 'response', { value: self._mockResponse, writable: true, configurable: true });
      self._responseHeaders = { 'content-type': 'application/json' };

      if (typeof self.onreadystatechange === 'function') {
        self.onreadystatechange(new Event('readystatechange'));
      }
      if (typeof self.onload === 'function') {
        self.onload(new ProgressEvent('load'));
      }
      if (typeof self.onloadend === 'function') {
        self.onloadend(new ProgressEvent('loadend'));
      }

      // Dispatch events for Angular's zone detection
      try {
        self.dispatchEvent(new Event('readystatechange'));
        self.dispatchEvent(new ProgressEvent('load'));
        self.dispatchEvent(new ProgressEvent('loadend'));
      } catch (e) {
        // dispatchEvent may not work on plain object
      }
    }, 15);
  };

  MockXHR.prototype.setRequestHeader = function (name, value) {
    this._requestHeaders[name.toLowerCase()] = value;
    if (!this._intercepted) {
      this._real.setRequestHeader(name, value);
    }
  };

  MockXHR.prototype.getResponseHeader = function (name) {
    if (this._intercepted) {
      return this._responseHeaders[name.toLowerCase()] || null;
    }
    return this._real.getResponseHeader(name);
  };

  MockXHR.prototype.getAllResponseHeaders = function () {
    if (this._intercepted) {
      var result = '';
      for (var key in this._responseHeaders) {
        result += key + ': ' + this._responseHeaders[key] + '\r\n';
      }
      return result;
    }
    return this._real.getAllResponseHeaders();
  };

  MockXHR.prototype.abort = function () {
    if (!this._intercepted) {
      this._real.abort();
    }
  };

  MockXHR.prototype.addEventListener = function () {
    if (!this._intercepted) {
      this._real.addEventListener.apply(this._real, arguments);
    } else {
      // Store listeners for intercepted requests
      if (!this._listeners) this._listeners = {};
      var type = arguments[0];
      var fn = arguments[1];
      if (!this._listeners[type]) this._listeners[type] = [];
      this._listeners[type].push(fn);
    }
  };

  MockXHR.prototype.removeEventListener = function () {
    if (!this._intercepted) {
      this._real.removeEventListener.apply(this._real, arguments);
    }
  };

  MockXHR.prototype.dispatchEvent = function (event) {
    if (this._listeners && this._listeners[event.type]) {
      var fns = this._listeners[event.type];
      for (var i = 0; i < fns.length; i++) {
        fns[i].call(this, event);
      }
    }
  };

  MockXHR.prototype.overrideMimeType = function () {
    if (!this._intercepted) {
      this._real.overrideMimeType.apply(this._real, arguments);
    }
  };

  // Proxy readonly properties from real XHR for non-intercepted requests
  ['readyState', 'status', 'statusText', 'responseText', 'response', 'responseType',
   'responseURL', 'responseXML', 'timeout', 'withCredentials'].forEach(function (prop) {
    var descriptor = {
      get: function () {
        if (this._intercepted) return undefined;
        return this._real[prop];
      },
      set: function (val) {
        if (!this._intercepted) {
          this._real[prop] = val;
        }
      },
      configurable: true
    };
    Object.defineProperty(MockXHR.prototype, prop, descriptor);
  });

  // Copy static properties
  MockXHR.UNSENT = 0;
  MockXHR.OPENED = 1;
  MockXHR.HEADERS_RECEIVED = 2;
  MockXHR.LOADING = 3;
  MockXHR.DONE = 4;

  // Replace global XMLHttpRequest
  window.XMLHttpRequest = MockXHR;

  console.log('[rustnzb demo] Mock API layer active');
})();
