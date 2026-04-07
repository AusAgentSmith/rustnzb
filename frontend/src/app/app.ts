import { Component, OnInit, OnDestroy, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { Router, RouterModule } from '@angular/router';
import { MatIconModule } from '@angular/material/icon';
import { ApiService } from './core/services/api.service';
import { AuthService } from './core/services/auth.service';
import { StatusResponse } from './core/models/queue.model';
import { AddNzbService } from './core/services/add-nzb.service';

@Component({
  selector: 'app-root',
  standalone: true,
  imports: [CommonModule, RouterModule, MatIconModule],
  template: `
    @if (!authenticated()) {
      <!-- Full-screen login (no sidebar) -->
      <router-outlet />
    } @else {
      <div class="shell">
        <!-- Sidebar -->
        <nav class="sidebar">
          <div class="sidebar-header">
            <div class="logo">
              <img src="logo.png" alt="rustnzb" class="logo-img" />
            </div>
            <div class="speed-widget">
              <div class="speed-value">{{ formatSpeed(speed()) }}</div>
              <div class="speed-label">Download Speed</div>
              <div class="speed-actions">
                <button class="speed-btn" (click)="togglePause()" [title]="paused() ? 'Resume' : 'Pause'">
                  <mat-icon>{{ paused() ? 'play_arrow' : 'pause' }}</mat-icon>
                </button>
              </div>
            </div>
          </div>

          <div class="sidebar-nav">
            <div class="nav-section">Downloads</div>
            <a class="nav-item" routerLink="/queue" routerLinkActive="active">
              <mat-icon>download</mat-icon> Queue
              @if (queueCount() > 0) { <span class="nav-badge">{{ queueCount() }}</span> }
            </a>
            <a class="nav-item" routerLink="/history" routerLinkActive="active">
              <mat-icon>history</mat-icon> History
            </a>

            <div class="nav-section">Automation</div>
            <a class="nav-item" routerLink="/rss" routerLinkActive="active">
              <mat-icon>rss_feed</mat-icon> RSS Feeds
            </a>
            <a class="nav-item" routerLink="/groups" routerLinkActive="active">
              <mat-icon>forum</mat-icon> Groups
            </a>

            <div class="nav-section">System</div>
            <a class="nav-item" routerLink="/settings" routerLinkActive="active">
              <mat-icon>settings</mat-icon> Settings
            </a>
            <a class="nav-item" routerLink="/logs" routerLinkActive="active">
              <mat-icon>terminal</mat-icon> Logs
            </a>
          </div>

          <div class="sidebar-footer">
            <div class="sidebar-stats">
              <div class="sidebar-stat">
                <span>Disk Free</span>
                <span class="sidebar-stat-value">{{ formatBytes(diskFree()) }}</span>
              </div>
              <div class="sidebar-stat">
                <span>Status</span>
                <span class="sidebar-stat-value connected">Connected</span>
              </div>
            </div>
            <button class="logout-btn" (click)="onLogout()">
              <mat-icon>logout</mat-icon> Sign out
            </button>
          </div>
        </nav>

        <!-- Main content -->
        <div class="content">
          <!-- Page header -->
          <div class="page-header">
            <div class="page-title">
              <mat-icon class="page-icon">{{ pageIcon() }}</mat-icon>
              {{ pageTitle() }}
            </div>
            <div class="page-actions">
              @if (isQueuePage()) {
                <button class="btn btn-primary" (click)="onAddNzb()">
                  <mat-icon>add</mat-icon> Add NZB
                </button>
                <button class="btn btn-icon" (click)="togglePause()" [title]="paused() ? 'Resume All' : 'Pause All'">
                  <mat-icon>{{ paused() ? 'play_arrow' : 'pause' }}</mat-icon>
                </button>
              }
            </div>
          </div>

          <!-- Router outlet -->
          <div class="page-content">
            <router-outlet />
          </div>
        </div>
      </div>
    }
  `,
  styles: [`
    :host { display: block; height: 100vh; overflow: hidden; }

    /* Shell layout */
    .shell { display: flex; height: 100vh; }

    /* Sidebar */
    .sidebar {
      width: 220px; background: #010409; border-right: 1px solid #21262d;
      display: flex; flex-direction: column; flex-shrink: 0; overflow: hidden;
    }
    .sidebar-header { padding: 16px 16px 16px; border-bottom: 1px solid #21262d; }
    .logo { display: flex; align-items: center; justify-content: center; }
    .logo-img { width: 180px; height: auto; }
    .speed-widget {
      margin-top: 14px; background: #161b22; border: 1px solid #21262d;
      border-radius: 8px; padding: 10px 12px; position: relative;
    }
    .speed-value {
      font-family: 'JetBrains Mono', Consolas, monospace;
      font-size: 20px; font-weight: 700; color: #3fb950;
    }
    .speed-label { font-size: 11px; color: #484f58; margin-top: 2px; }
    .speed-actions { position: absolute; top: 10px; right: 10px; }
    .speed-btn {
      background: none; border: 1px solid #30363d; color: #8b949e;
      border-radius: 4px; cursor: pointer; width: 28px; height: 28px;
      display: flex; align-items: center; justify-content: center; padding: 0;
    }
    .speed-btn:hover { background: #21262d; color: #c9d1d9; }
    .speed-btn mat-icon { font-size: 18px; width: 18px; height: 18px; }

    /* Nav */
    .sidebar-nav { flex: 1; padding: 8px 0; overflow-y: auto; }
    .nav-section {
      padding: 14px 16px 6px; font-size: 10px; font-weight: 600;
      color: #484f58; text-transform: uppercase; letter-spacing: 1px;
    }
    .nav-item {
      display: flex; align-items: center; gap: 10px; padding: 8px 16px;
      color: #8b949e; cursor: pointer; font-size: 13px; font-weight: 500;
      text-decoration: none; border-left: 2px solid transparent;
      transition: all 0.15s;
    }
    .nav-item:hover { color: #c9d1d9; background: #161b22; text-decoration: none; }
    .nav-item.active { color: #e6edf3; background: #161b22; border-left-color: #f0883e; }
    .nav-item mat-icon { font-size: 20px; width: 20px; height: 20px; }
    .nav-badge {
      margin-left: auto; background: #f0883e; color: #0d1117;
      font-size: 10px; font-weight: 700; padding: 1px 7px; border-radius: 10px;
    }

    /* Footer */
    .sidebar-footer { padding: 12px 16px; border-top: 1px solid #21262d; }
    .sidebar-stats { font-size: 11px; color: #484f58; display: flex; flex-direction: column; gap: 6px; margin-bottom: 10px; }
    .sidebar-stat { display: flex; justify-content: space-between; }
    .sidebar-stat-value { color: #8b949e; font-family: 'JetBrains Mono', Consolas, monospace; font-size: 11px; }
    .sidebar-stat-value.connected { color: #3fb950; }
    .logout-btn {
      display: flex; align-items: center; gap: 8px; width: 100%;
      padding: 6px 8px; border-radius: 4px; border: none;
      background: transparent; color: #484f58; cursor: pointer; font-size: 12px;
    }
    .logout-btn:hover { background: #161b22; color: #c9d1d9; }
    .logout-btn mat-icon { font-size: 16px; width: 16px; height: 16px; }

    /* Main content area */
    .content { flex: 1; display: flex; flex-direction: column; overflow: hidden; min-width: 0; }
    .page-header {
      display: flex; align-items: center; justify-content: space-between;
      padding: 14px 24px; border-bottom: 1px solid #21262d; background: #161b22; flex-shrink: 0;
    }
    .page-title {
      font-size: 18px; font-weight: 700; color: #e6edf3;
      display: flex; align-items: center; gap: 10px;
    }
    .page-icon { font-size: 24px; width: 24px; height: 24px; color: #f0883e; }
    .page-actions { display: flex; gap: 8px; }

    .btn {
      padding: 7px 16px; border-radius: 6px; border: 1px solid #30363d;
      background: #21262d; color: #c9d1d9; cursor: pointer; font-size: 13px;
      font-weight: 500; display: flex; align-items: center; gap: 6px; transition: all 0.15s;
    }
    .btn:hover { background: #30363d; }
    .btn-primary { background: #238636; border-color: #2ea043; color: white; }
    .btn-primary:hover { background: #2ea043; }
    .btn-icon { padding: 7px 8px; }
    .btn mat-icon { font-size: 18px; width: 18px; height: 18px; }

    .page-content { flex: 1; overflow: hidden; display: flex; flex-direction: column; }
  `],
})
export class App implements OnInit, OnDestroy {
  speed = signal(0);
  paused = signal(false);
  queueCount = signal(0);
  diskFree = signal(0);
  authenticated = signal(false);
  private pollTimer: ReturnType<typeof setInterval> | null = null;

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
  }

  ngOnDestroy(): void {
    if (this.pollTimer) clearInterval(this.pollTimer);
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
  }

  isQueuePage(): boolean {
    return this.router.url === '/queue';
  }

  pageTitle(): string {
    const url = this.router.url;
    if (url.startsWith('/queue')) return 'Queue';
    if (url.startsWith('/history')) return 'History';
    if (url.startsWith('/rss')) return 'RSS Feeds';
    if (url.startsWith('/groups')) return 'Groups';
    if (url.startsWith('/settings')) return 'Settings';
    if (url.startsWith('/logs')) return 'Logs';
    return 'Queue';
  }

  pageIcon(): string {
    const url = this.router.url;
    if (url.startsWith('/queue')) return 'download';
    if (url.startsWith('/history')) return 'history';
    if (url.startsWith('/rss')) return 'rss_feed';
    if (url.startsWith('/groups')) return 'forum';
    if (url.startsWith('/settings')) return 'settings';
    if (url.startsWith('/logs')) return 'terminal';
    return 'download';
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
