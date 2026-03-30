import { Component, OnInit, OnDestroy, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { Router, RouterModule } from '@angular/router';
import { MatIconModule } from '@angular/material/icon';
import { MatButtonModule } from '@angular/material/button';
import { MatBadgeModule } from '@angular/material/badge';
import { ApiService } from './core/services/api.service';
import { AuthService } from './core/services/auth.service';
import { StatusResponse } from './core/models/queue.model';

@Component({
  selector: 'app-root',
  standalone: true,
  imports: [CommonModule, RouterModule, MatIconModule, MatButtonModule, MatBadgeModule],
  template: `
    <!-- Top bar -->
    <div class="topbar">
      <div class="topbar-left">
        <span class="logo">⚡ rustnzbd</span>
      </div>
      <div class="topbar-right">
        @if (authenticated()) {
          <div class="speed-display">
            <span class="arrow">↓</span>
            <span>{{ formatSpeed(speed()) }}</span>
          </div>
          <button class="topbar-btn primary" routerLink="/queue">+ Add NZB</button>
          <button class="topbar-btn" (click)="togglePause()">
            {{ paused() ? 'Resume' : 'Pause' }}
          </button>
          <button class="topbar-btn" (click)="onLogout()">Logout</button>
        }
      </div>
    </div>

    <!-- Tabs -->
    @if (authenticated()) {
      <div class="tabs">
        <a class="tab" routerLink="/queue" routerLinkActive="active">
          Queue
          @if (queueCount() > 0) { <span class="badge">{{ queueCount() }}</span> }
        </a>
        <a class="tab" routerLink="/groups" routerLinkActive="active">Groups</a>
        <a class="tab" routerLink="/history" routerLinkActive="active">History</a>
        <a class="tab" routerLink="/rss" routerLinkActive="active">RSS</a>
        <a class="tab" routerLink="/settings" routerLinkActive="active">Settings</a>
        <a class="tab" routerLink="/logs" routerLinkActive="active">Logs</a>
      </div>
    }

    <!-- Content -->
    <div class="main">
      <router-outlet />
    </div>

    <!-- Status bar -->
    <div class="statusbar">
      @if (authenticated()) { <span class="status-ok">● Connected</span> }
      <span class="sep">|</span>
      <span>Disk: {{ formatBytes(diskFree()) }} free</span>
      @if (queueCount() > 0) {
        <span class="sep">|</span>
        <span>Queue: {{ queueCount() }} items</span>
      }
      <span class="spacer"></span>
      <span class="version">rustnzbd</span>
    </div>
  `,
  styles: [`
    :host { display: flex; flex-direction: column; height: 100vh; overflow: hidden; }

    .topbar {
      display: flex; align-items: center; justify-content: space-between;
      padding: 0 16px; height: 48px; background: #161b22;
      border-bottom: 1px solid #30363d;
    }
    .topbar-left { display: flex; align-items: center; gap: 16px; }
    .logo { font-size: 16px; font-weight: 700; color: #58a6ff; }
    .topbar-right { display: flex; align-items: center; gap: 12px; }
    .speed-display {
      display: flex; align-items: center; gap: 4px;
      background: #0d1117; padding: 4px 12px; border-radius: 4px;
      font-family: Consolas, monospace; font-size: 14px; font-weight: 600; color: #3fb950;
    }
    .arrow { font-size: 16px; }
    .topbar-btn {
      padding: 5px 12px; border-radius: 4px; border: 1px solid #30363d;
      background: #21262d; color: #c9d1d9; cursor: pointer; font-size: 12px;
    }
    .topbar-btn:hover { background: #30363d; }
    .topbar-btn.primary { background: #238636; border-color: #2ea043; color: white; }

    .tabs {
      display: flex; background: #161b22; border-bottom: 1px solid #30363d; padding: 0 16px;
    }
    .tab {
      padding: 10px 16px; cursor: pointer; color: #8b949e;
      border-bottom: 2px solid transparent; font-size: 13px;
      display: flex; align-items: center; gap: 6px; text-decoration: none;
    }
    .tab:hover { color: #c9d1d9; }
    .tab.active { color: #c9d1d9; border-bottom-color: #58a6ff; }
    .badge {
      background: #388bfd; color: white; font-size: 10px; font-weight: 700;
      padding: 1px 6px; border-radius: 10px;
    }

    .main { flex: 1; overflow: auto; }

    .statusbar {
      display: flex; align-items: center; gap: 8px;
      padding: 4px 16px; height: 24px; background: #161b22;
      border-top: 1px solid #30363d; font-size: 11px; color: #484f58;
    }
    .status-ok { color: #3fb950; }
    .sep { color: #30363d; }
    .spacer { flex: 1; }
    .version { color: #30363d; }
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
    if (!this.authenticated()) {
      return;
    }
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

  togglePause(): void {
    const action = this.paused() ? '/queue/resume' : '/queue/pause';
    this.api.post(action).subscribe(() => this.pollStatus());
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
