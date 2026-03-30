import { Component, OnInit, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MatSnackBar, MatSnackBarModule } from '@angular/material/snack-bar';
import { MatDialogModule, MatDialog } from '@angular/material/dialog';
import { ApiService } from '../../core/services/api.service';

interface ServerConfig {
  id: string; name: string; host: string; port: number; ssl: boolean; ssl_verify: boolean;
  username: string | null; password: string | null; connections: number; priority: number;
  enabled: boolean; retention: number; pipelining: number; optional: boolean; compress: boolean;
}

@Component({
  selector: 'app-settings-view',
  standalone: true,
  imports: [CommonModule, FormsModule, MatButtonModule, MatSnackBarModule, MatDialogModule],
  template: `
    <div class="settings-layout">
      <div class="nav">
        <div class="nav-item" [class.active]="tab === 'servers'" (click)="tab = 'servers'">Servers</div>
        <div class="nav-item" [class.active]="tab === 'general'" (click)="tab = 'general'">General</div>
      </div>
      <div class="content">
        @if (tab === 'servers') {
          <h3>News Servers</h3>
          @for (s of servers(); track s.id) {
            <div class="server-card">
              <div class="server-info">
                <div class="server-name">{{ s.name }} @if (s.optional) { <span class="optional">(backup)</span> }</div>
                <div class="server-host">{{ s.host }}:{{ s.port }} · {{ s.ssl ? 'SSL' : 'Plain' }} · {{ s.connections }} conn · Pipeline: {{ s.pipelining }}</div>
              </div>
              <div class="server-status">
                <span class="dot" [class.active]="s.enabled"></span>
                {{ s.enabled ? 'Enabled' : 'Disabled' }}
              </div>
              <button class="btn" (click)="testServer(s.id)">Test</button>
              <button class="btn" (click)="deleteServer(s.id)">✕</button>
            </div>
          }
          @if (servers().length === 0) {
            <div class="empty">No servers configured</div>
          }
        }
        @if (tab === 'general') {
          <h3>General Settings</h3>
          <div class="setting-row">
            <label>Speed Limit (bytes/sec, 0 = unlimited)</label>
            <input type="number" [(ngModel)]="speedLimit" class="setting-input" />
            <button class="btn" (click)="saveSpeedLimit()">Save</button>
          </div>
        }
      </div>
    </div>
  `,
  styles: [`
    :host { display: flex; height: 100%; }
    .settings-layout { display: flex; flex: 1; overflow: hidden; }
    .nav { width: 180px; border-right: 1px solid #21262d; background: #0d1117; padding: 8px 0; }
    .nav-item { padding: 8px 16px; cursor: pointer; font-size: 13px; color: #8b949e; }
    .nav-item:hover { color: #c9d1d9; background: #161b22; }
    .nav-item.active { color: #c9d1d9; background: rgba(56,139,253,0.1); border-right: 2px solid #58a6ff; }
    .content { flex: 1; padding: 20px 24px; overflow-y: auto; }
    h3 { font-size: 16px; margin-bottom: 12px; }
    .server-card { display: flex; align-items: center; gap: 12px; padding: 12px 16px; background: #161b22; border: 1px solid #30363d; border-radius: 6px; margin-bottom: 8px; }
    .server-info { flex: 1; }
    .server-name { font-weight: 600; font-size: 14px; }
    .optional { color: #8b949e; font-weight: normal; font-size: 12px; }
    .server-host { font-size: 12px; color: #8b949e; margin-top: 2px; }
    .server-status { display: flex; align-items: center; gap: 4px; font-size: 12px; color: #8b949e; }
    .dot { width: 8px; height: 8px; border-radius: 50%; background: #484f58; }
    .dot.active { background: #3fb950; }
    .btn { padding: 4px 10px; border-radius: 4px; border: 1px solid #30363d; background: #21262d; color: #c9d1d9; cursor: pointer; font-size: 12px; }
    .btn:hover { background: #30363d; }
    .empty { padding: 24px; color: #484f58; text-align: center; }
    .setting-row { display: flex; align-items: center; gap: 12px; margin-bottom: 12px; }
    .setting-row label { font-size: 13px; min-width: 250px; }
    .setting-input { padding: 6px 10px; background: #161b22; border: 1px solid #30363d; border-radius: 4px; color: #c9d1d9; font-size: 13px; width: 150px; }
  `],
})
export class SettingsViewComponent implements OnInit {
  tab = 'servers';
  servers = signal<ServerConfig[]>([]);
  speedLimit = 0;

  constructor(private api: ApiService, private snack: MatSnackBar) {}

  ngOnInit(): void { this.loadServers(); this.loadSpeedLimit(); }

  loadServers(): void {
    this.api.get<{ servers: ServerConfig[] }>('/config/servers').subscribe({
      next: r => this.servers.set(r.servers || []),
      error: () => {},
    });
  }

  testServer(id: string): void {
    this.api.post<{ success: boolean; message: string }>(`/config/servers/${id}/test`).subscribe({
      next: r => this.snack.open(r.message, 'Close', { duration: 3000 }),
      error: () => this.snack.open('Test failed', 'Close', { duration: 3000 }),
    });
  }

  deleteServer(id: string): void {
    this.api.delete(`/config/servers/${id}`).subscribe(() => { this.loadServers(); this.snack.open('Server removed', 'Close', { duration: 2000 }); });
  }

  loadSpeedLimit(): void {
    this.api.get<{ speed_limit_bps: number }>('/config/speed-limit').subscribe({
      next: r => this.speedLimit = r.speed_limit_bps,
      error: () => {},
    });
  }

  saveSpeedLimit(): void {
    this.api.put('/config/speed-limit', { speed_limit_bps: this.speedLimit }).subscribe(() => {
      this.snack.open('Speed limit saved', 'Close', { duration: 2000 });
    });
  }
}
