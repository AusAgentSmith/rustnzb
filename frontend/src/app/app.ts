import { Component, OnInit, OnDestroy, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Router, RouterModule } from '@angular/router';
import { ApiService } from './core/services/api.service';
import { AuthService } from './core/services/auth.service';
import { StatusResponse } from './core/models/queue.model';
import { AddNzbService } from './core/services/add-nzb.service';

@Component({
  selector: 'app-root',
  standalone: true,
  imports: [CommonModule, FormsModule, RouterModule],
  template: `
    @if (!authenticated()) {
      <!-- Full-screen login (no chrome) -->
      <router-outlet />
    } @else {
      <div class="shell">
        <header>
          <div>
            <span class="brand">rust<span>nzb</span></span>
            <span class="ver">v{{ version }}</span>
          </div>
          <div class="status">
            <span class="pill" [class.ok]="!paused()" [class.warn]="paused()">
              ● {{ paused() ? 'Paused' : 'Daemon running' }}
            </span>
            <span class="pill">Speed: <b>{{ formatSpeed(speed()) }}</b></span>
            <span class="pill">Queue: <b>{{ queueCount() }}</b></span>
            <span class="pill">Free: <b>{{ formatBytes(diskFree()) }}</b></span>
          </div>
        </header>

        <nav class="top">
          <a routerLink="/queue"    routerLinkActive="active">Queue</a>
          <a routerLink="/history"  routerLinkActive="active">History</a>
          <a routerLink="/groups"   routerLinkActive="active">Search</a>
          <a routerLink="/rss"      routerLinkActive="active">RSS</a>
          <a routerLink="/logs"     routerLinkActive="active">Logs</a>
          <a routerLink="/settings" routerLinkActive="active">Settings</a>
          <div class="spacer"></div>
          <button class="action primary" (click)="onAddNzb()">+ Upload NZB</button>
          <div class="pause-group">
            <button class="action" (click)="togglePause()">
              {{ paused() ? '▶ Resume all' : '❚❚ Pause all' }}
            </button>
            @if (!paused()) {
              <button class="action pause-caret" (click)="pauseMenuOpen = !pauseMenuOpen" title="Pause for…">▾</button>
              @if (pauseMenuOpen) {
                <div class="pause-menu" (click)="$event.stopPropagation()">
                  <div class="pm-title">Pause for…</div>
                  @for (opt of pauseTimerOptions; track opt.secs) {
                    <button class="pm-item" (click)="pauseFor(opt.secs)">{{ opt.label }}</button>
                  }
                  <div class="pm-custom">
                    <input type="number" min="1" placeholder="min" [(ngModel)]="customPauseMin"
                           (keydown.enter)="pauseForCustom()" />
                    <button class="pm-go" (click)="pauseForCustom()">Go</button>
                  </div>
                </div>
              }
            }
          </div>
          <button class="action muted" (click)="onLogout()" title="Sign out">Sign out</button>
        </nav>

        <main>
          <router-outlet />
        </main>
      </div>
    }
  `,
  styles: [`
    :host { display: block; height: 100vh; overflow: hidden; }

    .shell { display: flex; flex-direction: column; height: 100vh; }

    /* ---- Header ---- */
    header {
      display: flex; align-items: center; justify-content: space-between;
      padding: 12px 20px;
      background: var(--panel);
      border-bottom: 1px solid var(--line);
      flex-shrink: 0;
    }
    .brand { font-weight: 700; font-size: 16px; letter-spacing: .2px; }
    .brand span { color: var(--accent); }
    .ver { color: var(--mute); font-size: 11px; margin-left: 8px; font-weight: 400; }
    .status { display: flex; gap: 10px; align-items: center; font-size: 12px; }

    /* ---- Top nav ---- */
    nav.top {
      display: flex;
      padding: 0 20px;
      background: var(--panel);
      border-bottom: 1px solid var(--line);
      flex-shrink: 0;
      overflow-x: auto;
      align-items: center;
    }
    nav.top a {
      color: var(--mute);
      padding: 10px 16px;
      border-bottom: 2px solid transparent;
      text-decoration: none;
      font-size: 14px;
      white-space: nowrap;
      transition: color .15s;
    }
    nav.top a:hover { color: var(--text); text-decoration: none; }
    nav.top a.active { color: var(--text); border-bottom-color: var(--accent); }

    nav.top .spacer { flex: 1; }
    nav.top .action {
      background: none; border: none;
      color: var(--text); padding: 10px 14px;
      cursor: pointer; font: inherit; font-size: 13px;
      opacity: .85;
    }
    nav.top .action:hover { opacity: 1; }
    nav.top .action.primary { color: var(--accent2); font-weight: 600; }
    nav.top .action.muted { color: var(--mute); font-size: 12px; }

    /* Pause split-button + dropdown */
    .pause-group { position: relative; display: flex; align-items: center; }
    .pause-caret {
      padding: 10px 6px !important;
      font-size: 11px !important;
      margin-left: -6px;
    }
    .pause-menu {
      position: absolute;
      top: 100%;
      right: 0;
      margin-top: 4px;
      background: var(--panel);
      border: 1px solid var(--line);
      border-radius: 6px;
      box-shadow: 0 8px 24px rgba(0,0,0,.35);
      padding: 6px;
      min-width: 160px;
      z-index: 40;
    }
    .pm-title { font-size: 11px; color: var(--mute); padding: 4px 8px 6px; text-transform: uppercase; letter-spacing: .4px; }
    .pm-item {
      display: block; width: 100%; text-align: left;
      background: none; border: none; color: var(--text);
      padding: 6px 10px; border-radius: 4px; cursor: pointer;
      font: inherit; font-size: 13px;
    }
    .pm-item:hover { background: var(--panel2); }
    .pm-custom {
      display: flex; gap: 4px; padding: 6px 4px 2px;
      border-top: 1px solid var(--line); margin-top: 4px;
    }
    .pm-custom input {
      flex: 1; min-width: 0; background: var(--panel2);
      border: 1px solid var(--line); color: var(--text);
      padding: 5px 8px; border-radius: 4px; font: inherit; font-size: 12px;
      outline: none;
    }
    .pm-go {
      background: var(--accent); color: #fff; border: none;
      padding: 5px 10px; border-radius: 4px; cursor: pointer;
      font: inherit; font-size: 12px;
    }

    /* ---- Main area ---- */
    main {
      flex: 1;
      overflow-y: auto;
      padding: 20px;
      max-width: 1320px;
      margin: 0 auto;
      width: 100%;
      box-sizing: border-box;
    }
  `],
})
export class App implements OnInit, OnDestroy {
  // Version string shown in the header. Kept in sync with package.json manually.
  readonly version = '0.2.4';

  speed = signal(0);
  paused = signal(false);
  queueCount = signal(0);
  diskFree = signal(0);
  authenticated = signal(false);
  pauseMenuOpen = false;
  customPauseMin: number | null = null;
  readonly pauseTimerOptions = [
    { label: '5 minutes', secs: 5 * 60 },
    { label: '15 minutes', secs: 15 * 60 },
    { label: '30 minutes', secs: 30 * 60 },
    { label: '1 hour', secs: 60 * 60 },
    { label: '2 hours', secs: 2 * 60 * 60 },
  ];
  private pollTimer: ReturnType<typeof setInterval> | null = null;
  private docClickHandler = (e: MouseEvent) => {
    if (!this.pauseMenuOpen) return;
    const el = (e.target as HTMLElement).closest('.pause-group');
    if (!el) this.pauseMenuOpen = false;
  };

  constructor(
    private api: ApiService,
    private authService: AuthService,
    private router: Router,
    private addNzbService: AddNzbService,
  ) {}

  ngOnInit(): void {
    this.authenticated.set(this.authService.isLoggedIn());
    this.pollStatus();
    this.pollTimer = setInterval(() => this.pollStatus(), 2000);
    document.addEventListener('click', this.docClickHandler);
  }

  ngOnDestroy(): void {
    if (this.pollTimer) clearInterval(this.pollTimer);
    document.removeEventListener('click', this.docClickHandler);
  }

  pollStatus(): void {
    this.authenticated.set(this.authService.isLoggedIn());
    if (!this.authenticated()) return;
    this.api.get<StatusResponse>('/status').subscribe({
      next: (s) => {
        this.speed.set(s.speed_bps);
        this.paused.set(s.paused);
        this.queueCount.set(s.queue_size);
        this.diskFree.set(s.disk_free_bytes);
      },
      error: () => {},
    });
  }

  onLogout(): void {
    this.authenticated.set(false);
    this.authService.logout().subscribe({
      complete: () => this.router.navigate(['/login']),
      error: () => this.router.navigate(['/login']),
    });
  }

  onAddNzb(): void {
    if (this.router.url !== '/queue') {
      this.router.navigate(['/queue']).then(() => this.addNzbService.togglePanel());
    } else {
      this.addNzbService.togglePanel();
    }
  }

  togglePause(): void {
    const action = this.paused() ? '/queue/resume' : '/queue/pause';
    this.api.post(action).subscribe(() => this.pollStatus());
    this.pauseMenuOpen = false;
  }

  pauseFor(secs: number): void {
    this.api.post(`/queue/pause-for?duration_secs=${secs}`).subscribe(() => this.pollStatus());
    this.pauseMenuOpen = false;
  }

  pauseForCustom(): void {
    const mins = this.customPauseMin;
    if (!mins || mins <= 0) return;
    this.pauseFor(Math.round(mins * 60));
    this.customPauseMin = null;
  }

  formatSpeed(bps: number): string {
    if (bps === 0) return '0 B/s';
    const k = 1024;
    const sizes = ['B/s', 'KB/s', 'MB/s', 'GB/s'];
    const i = Math.floor(Math.log(bps) / Math.log(k));
    return parseFloat((bps / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
  }

  formatBytes(bytes: number): string {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
  }
}
