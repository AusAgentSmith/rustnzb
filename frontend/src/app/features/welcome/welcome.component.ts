import { Component, OnInit, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Router } from '@angular/router';
import { ApiService } from '../../core/services/api.service';

type Step = 'landing' | 'connect' | 'preview' | 'applying';
type ImportMethod = 'api' | 'ini';

interface ImportedServer {
  name: string;
  host: string;
  port: number;
  ssl: boolean;
  ssl_verify: boolean;
  username: string | null;
  password: string | null;
  password_masked: boolean;
  connections: number;
  priority: number;
  enabled: boolean;
  retention: number;
  optional: boolean;
}

interface ImportedGeneral {
  api_key: string | null;
  complete_dir: string | null;
  incomplete_dir: string | null;
  speed_limit_bps: number;
}

interface CategoryConfig {
  name: string;
  output_dir: string | null;
  post_processing: number;
}

interface RssFeedConfig {
  name: string;
  url: string;
  poll_interval_secs: number;
  category: string | null;
}

interface ImportPreview {
  servers: ImportedServer[];
  categories: CategoryConfig[];
  general: ImportedGeneral;
  rss_feeds: RssFeedConfig[];
  warnings: string[];
  skipped_fields: string[];
}

@Component({
  selector: 'app-welcome',
  standalone: true,
  imports: [CommonModule, FormsModule],
  template: `
    <div class="welcome-wrapper">
      <div class="welcome-card" [class.wide]="step() === 'preview'">

        <!-- ── LANDING ── -->
        @if (step() === 'landing') {
          <div class="card-header">
            <img src="/logo.png" alt="rustnzb" class="logo" />
            <h1>Welcome to rustnzb</h1>
            <p class="subtitle">Let's get you set up. You can import your settings from an existing SABnzbd instance, or configure everything manually.</p>
          </div>

          <div class="landing-actions">
            <button class="btn primary large" (click)="step.set('connect')">
              <span class="btn-icon">⇄</span>
              Import from SABnzbd
            </button>
            <button class="btn secondary large" (click)="skipToSettings()">
              Set up manually
            </button>
          </div>

          <div class="skip-link">
            <button class="link-btn" (click)="skipToQueue()">Skip for now →</button>
          </div>
        }

        <!-- ── CONNECT ── -->
        @if (step() === 'connect') {
          <div class="card-header">
            <h2>Import from SABnzbd</h2>
            <p class="subtitle">Connect to a running SABnzbd instance or upload its config file.</p>
          </div>

          <div class="method-tabs">
            <button [class.active]="method() === 'api'" (click)="method.set('api')">Live instance</button>
            <button [class.active]="method() === 'ini'" (click)="method.set('ini')">Config file (.ini)</button>
          </div>

          @if (method() === 'api') {
            <div class="form-section">
              <div class="form-group">
                <label class="form-label">SABnzbd URL</label>
                <input
                  type="text"
                  class="form-input"
                  [(ngModel)]="sabnzbdUrl"
                  placeholder="http://localhost:8080"
                />
              </div>
              <div class="form-group">
                <label class="form-label">API Key</label>
                <input
                  type="text"
                  class="form-input mono"
                  [(ngModel)]="sabnzbdApiKey"
                  placeholder="32-character hex key"
                  autocomplete="off"
                />
                <div class="hint">Found in SABnzbd → Config → General → API Key</div>
              </div>
            </div>
          }

          @if (method() === 'ini') {
            <div class="form-section">
              <div class="form-group">
                <label class="form-label">sabnzbd.ini file</label>
                <input
                  type="file"
                  class="form-input file-input"
                  accept=".ini"
                  (change)="onFileSelected($event)"
                />
                <div class="hint">Usually at <code>~/.sabnzbd/sabnzbd.ini</code> or in the SABnzbd config directory</div>
              </div>
            </div>
          }

          @if (connectError()) {
            <div class="error-box">{{ connectError() }}</div>
          }

          <div class="btn-row">
            <button class="btn ghost" (click)="step.set('landing')">← Back</button>
            <button
              class="btn primary"
              (click)="fetchConfig()"
              [disabled]="fetching()"
            >
              @if (fetching()) { Fetching… } @else { Fetch config }
            </button>
          </div>
        }

        <!-- ── PREVIEW ── -->
        @if (step() === 'preview' && preview()) {
          <div class="card-header">
            <h2>Review imported settings</h2>
            <p class="subtitle">
              Check the settings below before applying. Server passwords from the live API are masked — enter them manually if needed.
            </p>
          </div>

          @if (preview()!.warnings.length > 0) {
            <div class="warning-box">
              <div class="warning-title">Warnings</div>
              @for (w of preview()!.warnings; track w) {
                <div class="warning-item">⚠ {{ w }}</div>
              }
            </div>
          }

          <!-- Servers -->
          <div class="preview-section">
            <div class="preview-section-title">
              News servers
              <span class="badge">{{ preview()!.servers.length }}</span>
            </div>
            @if (preview()!.servers.length === 0) {
              <div class="empty-note">No servers found</div>
            }
            @for (server of preview()!.servers; track server.host; let i = $index) {
              <div class="preview-item server-item">
                <div class="server-row">
                  <div class="server-info">
                    <span class="server-name">{{ server.name || server.host }}</span>
                    <span class="server-meta">
                      {{ server.host }}:{{ server.port }}
                      · {{ server.connections }} conn
                      @if (server.ssl) { · SSL }
                      @if (server.optional) { · optional }
                    </span>
                  </div>
                  <div class="server-status" [class.enabled]="server.enabled" [class.disabled]="!server.enabled">
                    {{ server.enabled ? 'enabled' : 'disabled' }}
                  </div>
                </div>
                @if (server.password_masked) {
                  <div class="password-row">
                    <label class="form-label small">Password (masked in SABnzbd API — enter if needed)</label>
                    <input
                      type="password"
                      class="form-input"
                      [ngModel]="server.password ?? ''"
                      (ngModelChange)="setServerPassword(i, $event)"
                      placeholder="Leave blank if not required"
                      autocomplete="new-password"
                    />
                  </div>
                }
              </div>
            }
          </div>

          <!-- Categories -->
          @if (preview()!.categories.length > 0) {
            <div class="preview-section">
              <div class="preview-section-title">
                Categories
                <span class="badge">{{ preview()!.categories.length }}</span>
              </div>
              @for (cat of preview()!.categories; track cat.name) {
                <div class="preview-item">
                  <span class="cat-name">{{ cat.name }}</span>
                  @if (cat.output_dir) {
                    <span class="cat-dir">→ {{ cat.output_dir }}</span>
                  }
                </div>
              }
            </div>
          }

          <!-- RSS Feeds -->
          @if (preview()!.rss_feeds.length > 0) {
            <div class="preview-section">
              <div class="preview-section-title">
                RSS feeds
                <span class="badge">{{ preview()!.rss_feeds.length }}</span>
              </div>
              @for (feed of preview()!.rss_feeds; track feed.name) {
                <div class="preview-item">
                  <span class="cat-name">{{ feed.name }}</span>
                  <span class="cat-dir">{{ feed.url }}</span>
                </div>
              }
            </div>
          }

          @if (applyError()) {
            <div class="error-box">{{ applyError() }}</div>
          }

          <div class="btn-row">
            <button class="btn ghost" (click)="step.set('connect')">← Back</button>
            <button class="btn secondary" (click)="skipToQueue()">Skip import</button>
            <button class="btn primary" (click)="applyImport()" [disabled]="hasMaskedPasswords()">
              Apply & continue
            </button>
          </div>

          @if (hasMaskedPasswords()) {
            <div class="hint center">Enter passwords for all masked servers before applying, or remove them.</div>
          }
        }

        <!-- ── APPLYING ── -->
        @if (step() === 'applying') {
          <div class="card-header centered">
            <div class="spinner"></div>
            <h2>Applying configuration…</h2>
            <p class="subtitle">Writing servers, categories, and feeds to your config.</p>
          </div>
        }

      </div>
    </div>
  `,
  styles: [`
    .welcome-wrapper {
      display: flex; align-items: flex-start; justify-content: center;
      min-height: 100vh; background: #0d1117; padding: 40px 16px;
    }

    .welcome-card {
      width: 100%; max-width: 520px; background: #161b22;
      border: 1px solid #30363d; border-radius: 8px; padding: 32px;
      display: flex; flex-direction: column; gap: 24px;
    }
    .welcome-card.wide { max-width: 720px; }

    .card-header { display: flex; flex-direction: column; gap: 8px; }
    .card-header.centered { align-items: center; text-align: center; }

    .logo { width: 180px; height: auto; margin-bottom: 8px; }

    h1 { font-size: 22px; font-weight: 700; color: #e6edf3; margin: 0; }
    h2 { font-size: 18px; font-weight: 700; color: #e6edf3; margin: 0; }

    .subtitle { color: #8b949e; font-size: 14px; margin: 0; line-height: 1.5; }

    .landing-actions { display: flex; flex-direction: column; gap: 12px; }

    .btn {
      padding: 10px 20px; border-radius: 6px; font-size: 14px; font-weight: 600;
      cursor: pointer; border: 1px solid transparent; transition: background 0.15s, border-color 0.15s;
      display: inline-flex; align-items: center; gap: 8px; justify-content: center;
    }
    .btn:disabled { opacity: 0.5; cursor: not-allowed; }
    .btn.primary { background: #238636; border-color: #2ea043; color: #fff; }
    .btn.primary:hover:not(:disabled) { background: #2ea043; }
    .btn.secondary { background: #21262d; border-color: #30363d; color: #c9d1d9; }
    .btn.secondary:hover:not(:disabled) { background: #30363d; }
    .btn.ghost { background: transparent; border-color: #30363d; color: #8b949e; }
    .btn.ghost:hover:not(:disabled) { border-color: #58a6ff; color: #58a6ff; }
    .btn.large { padding: 14px 24px; font-size: 15px; }

    .btn-icon { font-size: 16px; }

    .skip-link { text-align: center; }
    .link-btn {
      background: none; border: none; color: #8b949e; font-size: 13px;
      cursor: pointer; text-decoration: underline; padding: 0;
    }
    .link-btn:hover { color: #58a6ff; }

    .method-tabs { display: flex; gap: 0; border-bottom: 1px solid #30363d; }
    .method-tabs button {
      background: none; border: none; border-bottom: 2px solid transparent;
      color: #8b949e; padding: 8px 16px; font-size: 13px; font-weight: 600;
      cursor: pointer; margin-bottom: -1px;
    }
    .method-tabs button.active { color: #58a6ff; border-bottom-color: #58a6ff; }
    .method-tabs button:hover:not(.active) { color: #c9d1d9; }

    .form-section { display: flex; flex-direction: column; gap: 16px; }
    .form-group { display: flex; flex-direction: column; gap: 6px; }
    .form-label { color: #c9d1d9; font-size: 13px; font-weight: 600; }
    .form-label.small { font-size: 12px; font-weight: 400; color: #8b949e; }
    .form-input {
      background: #0d1117; border: 1px solid #30363d; border-radius: 6px;
      padding: 8px 12px; color: #c9d1d9; font-size: 14px; outline: none;
    }
    .form-input:focus { border-color: #58a6ff; }
    .form-input::placeholder { color: #484f58; }
    .form-input.mono { font-family: monospace; letter-spacing: 0.05em; }
    .file-input { padding: 6px 8px; }

    .hint { color: #6e7681; font-size: 12px; }
    .hint.center { text-align: center; }
    code { background: #21262d; padding: 1px 4px; border-radius: 3px; font-size: 11px; }

    .error-box {
      background: rgba(248,81,73,0.1); border: 1px solid #f85149;
      border-radius: 6px; padding: 10px 14px; color: #f85149; font-size: 13px;
    }

    .warning-box {
      background: rgba(210,153,34,0.1); border: 1px solid #d2993a;
      border-radius: 6px; padding: 12px 14px; display: flex; flex-direction: column; gap: 4px;
    }
    .warning-title { color: #d2993a; font-size: 12px; font-weight: 700; text-transform: uppercase; letter-spacing: 0.05em; }
    .warning-item { color: #e3b341; font-size: 13px; }

    .btn-row { display: flex; gap: 8px; justify-content: flex-end; align-items: center; }

    .preview-section { display: flex; flex-direction: column; gap: 8px; }
    .preview-section-title {
      font-size: 13px; font-weight: 700; color: #8b949e;
      text-transform: uppercase; letter-spacing: 0.05em;
      display: flex; align-items: center; gap: 8px;
    }
    .badge {
      background: #21262d; border: 1px solid #30363d; border-radius: 10px;
      padding: 1px 7px; font-size: 11px; font-weight: 600; color: #8b949e;
    }

    .preview-item {
      background: #0d1117; border: 1px solid #21262d; border-radius: 6px;
      padding: 10px 12px; display: flex; flex-direction: column; gap: 6px;
    }

    .server-row { display: flex; justify-content: space-between; align-items: flex-start; gap: 8px; }
    .server-info { display: flex; flex-direction: column; gap: 2px; }
    .server-name { color: #e6edf3; font-size: 14px; font-weight: 600; }
    .server-meta { color: #8b949e; font-size: 12px; }
    .server-status { font-size: 11px; font-weight: 600; padding: 2px 7px; border-radius: 10px; }
    .server-status.enabled { background: rgba(35,134,54,0.2); color: #3fb950; }
    .server-status.disabled { background: rgba(139,148,158,0.1); color: #6e7681; }

    .password-row { display: flex; flex-direction: column; gap: 4px; }

    .cat-name { color: #c9d1d9; font-size: 13px; font-weight: 600; }
    .cat-dir { color: #6e7681; font-size: 12px; font-family: monospace; }

    .empty-note { color: #484f58; font-size: 13px; font-style: italic; }

    .spinner {
      width: 36px; height: 36px; border: 3px solid #21262d;
      border-top-color: #58a6ff; border-radius: 50%;
      animation: spin 0.8s linear infinite; margin: 0 auto 16px;
    }
    @keyframes spin { to { transform: rotate(360deg); } }
  `],
})
export class WelcomeComponent implements OnInit {
  step = signal<Step>('landing');
  method = signal<ImportMethod>('api');

  sabnzbdUrl = '';
  sabnzbdApiKey = '';
  selectedFile: File | null = null;

  fetching = signal(false);
  connectError = signal('');

  preview = signal<ImportPreview | null>(null);
  applyError = signal('');

  constructor(
    private api: ApiService,
    private router: Router,
  ) {}

  ngOnInit(): void {
    this.api.get<{ has_servers: boolean }>('/setup/status').subscribe({
      next: (status) => {
        if (status.has_servers) {
          this.router.navigate(['/queue']);
        }
      },
      error: () => {},
    });
  }

  skipToQueue(): void {
    this.router.navigate(['/queue']);
  }

  skipToSettings(): void {
    this.router.navigate(['/settings']);
  }

  onFileSelected(event: Event): void {
    const input = event.target as HTMLInputElement;
    this.selectedFile = input.files?.[0] ?? null;
  }

  fetchConfig(): void {
    this.connectError.set('');

    if (this.method() === 'api') {
      if (!this.sabnzbdUrl.trim()) {
        this.connectError.set('SABnzbd URL is required.');
        return;
      }
      if (!this.sabnzbdApiKey.trim()) {
        this.connectError.set('API key is required.');
        return;
      }

      this.fetching.set(true);
      this.api.post<ImportPreview>('/setup/import-sabnzbd-api', {
        url: this.sabnzbdUrl.trim(),
        api_key: this.sabnzbdApiKey.trim(),
      }).subscribe({
        next: (p) => {
          this.fetching.set(false);
          this.preview.set(p);
          this.step.set('preview');
        },
        error: (err) => {
          this.fetching.set(false);
          this.connectError.set(
            err.error?.message ?? err.error?.error ?? 'Failed to connect to SABnzbd. Check the URL and API key.'
          );
        },
      });
    } else {
      if (!this.selectedFile) {
        this.connectError.set('Please select a sabnzbd.ini file.');
        return;
      }

      this.fetching.set(true);
      const form = new FormData();
      form.append('file', this.selectedFile, this.selectedFile.name);

      // ApiService doesn't have postForm, so use HttpClient directly via a POST observable
      this.api.postForm<ImportPreview>('/setup/import-sabnzbd', form).subscribe({
        next: (p) => {
          this.fetching.set(false);
          this.preview.set(p);
          this.step.set('preview');
        },
        error: (err) => {
          this.fetching.set(false);
          this.connectError.set(
            err.error?.message ?? err.error?.error ?? 'Failed to parse the ini file.'
          );
        },
      });
    }
  }

  setServerPassword(index: number, password: string): void {
    const p = this.preview();
    if (!p) return;
    const servers = [...p.servers];
    servers[index] = {
      ...servers[index],
      password: password || null,
      password_masked: false,
    };
    this.preview.set({ ...p, servers });
  }

  hasMaskedPasswords(): boolean {
    return this.preview()?.servers.some(s => s.password_masked) ?? false;
  }

  applyImport(): void {
    const p = this.preview();
    if (!p) return;

    this.applyError.set('');
    this.step.set('applying');

    this.api.post<{ status: boolean }>('/setup/apply', p).subscribe({
      next: () => {
        this.router.navigate(['/queue']);
      },
      error: (err) => {
        this.applyError.set(
          err.error?.message ?? err.error?.error ?? 'Failed to apply settings.'
        );
        this.step.set('preview');
      },
    });
  }
}
