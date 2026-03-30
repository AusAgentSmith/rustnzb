import { Component, OnInit, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { MatButtonModule } from '@angular/material/button';
import { MatSnackBar, MatSnackBarModule } from '@angular/material/snack-bar';
import { ApiService } from '../../core/services/api.service';
import { HistoryEntry } from '../../core/models/queue.model';

interface LogEntry {
  seq: number;
  timestamp: string;
  level: string;
  message: string;
}

interface LogsResponse {
  entries: LogEntry[];
  latest_seq: number;
}

@Component({
  selector: 'app-history-view',
  standalone: true,
  imports: [CommonModule, MatButtonModule, MatSnackBarModule],
  template: `
    <div class="toolbar">
      <span class="title">Download History</span>
      <span class="spacer"></span>
      @if (entries().length > 0) {
        <button class="btn warn" (click)="clearAll()">Clear All</button>
      }
    </div>
    <div class="list">
      @for (e of entries(); track e.id) {
        <div class="item-wrapper">
          <div class="item">
            <div class="icon">{{ e.status === 'completed' ? '✅' : '❌' }}</div>
            <div class="info">
              <div class="name">{{ e.name }}</div>
              <div class="meta">
                <span class="tag" [class]="'tag-' + e.status">{{ e.status | uppercase }}</span>
                <span>{{ formatBytes(e.total_bytes) }}</span>
                @if (e.category) { <span>{{ e.category }}</span> }
                @for (s of e.stages; track s.name) {
                  <span>{{ s.name }}: {{ s.status }}</span>
                }
                @if (e.error_message) { <span class="error-msg">{{ e.error_message }}</span> }
                <span>{{ e.completed_at }}</span>
              </div>
            </div>
            <div class="actions">
              <button class="btn" (click)="toggleLogs(e.id)">Logs</button>
              @if (e.status === 'failed') {
                <button class="btn" (click)="retry(e.id)">🔄 Retry</button>
              }
              <button class="btn" (click)="remove(e.id)">✕</button>
            </div>
          </div>
          @if (expandedLogs()[e.id]) {
            <div class="log-viewer">
              @if (expandedLogs()[e.id]!.loading) {
                <div class="log-loading">Loading logs...</div>
              } @else if (expandedLogs()[e.id]!.entries.length === 0) {
                <div class="log-empty">No log entries</div>
              } @else {
                @for (log of expandedLogs()[e.id]!.entries; track log.seq) {
                  <div class="log-line" [class]="'log-' + log.level.toLowerCase()">
                    <span class="log-ts">{{ log.timestamp }}</span>
                    <span class="log-msg">{{ log.message }}</span>
                  </div>
                }
              }
            </div>
          }
        </div>
      }
      @if (entries().length === 0) {
        <div class="empty"><p>No download history</p></div>
      }
    </div>
  `,
  styles: [`
    :host { display: flex; flex-direction: column; height: 100%; }
    .toolbar { display: flex; align-items: center; padding: 10px 16px; background: #0d1117; border-bottom: 1px solid #21262d; }
    .title { font-size: 14px; font-weight: 600; }
    .spacer { flex: 1; }
    .btn { padding: 4px 10px; border-radius: 4px; border: 1px solid #30363d; background: #21262d; color: #c9d1d9; cursor: pointer; font-size: 12px; }
    .btn:hover { background: #30363d; }
    .btn.warn { background: #da3633; border-color: #f85149; color: white; }
    .list { flex: 1; overflow-y: auto; }
    .item-wrapper { border-bottom: 1px solid #21262d; }
    .item { display: flex; align-items: center; gap: 12px; padding: 10px 16px; }
    .item:hover { background: #161b22; }
    .icon { font-size: 18px; width: 24px; text-align: center; }
    .info { flex: 1; min-width: 0; }
    .name { font-weight: 600; font-size: 13px; margin-bottom: 3px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .meta { display: flex; gap: 10px; font-size: 11px; color: #8b949e; flex-wrap: wrap; }
    .tag { padding: 1px 6px; border-radius: 3px; font-size: 10px; font-weight: 600; }
    .tag-completed { background: #1a3a1a; color: #3fb950; }
    .tag-failed { background: #3d1418; color: #f85149; }
    .error-msg { color: #f85149; }
    .actions { display: flex; gap: 4px; }
    .empty { text-align: center; padding: 48px; color: #484f58; }
    .log-viewer { background: #0d1117; padding: 12px; font-family: monospace; font-size: 11px; max-height: 300px; overflow-y: auto; border-top: 1px solid #30363d; margin: 0 16px 8px 16px; border-radius: 4px; border: 1px solid #30363d; }
    .log-loading, .log-empty { color: #484f58; padding: 8px 0; }
    .log-line { display: flex; gap: 8px; line-height: 1.6; white-space: pre-wrap; word-break: break-all; }
    .log-ts { color: #484f58; flex-shrink: 0; }
    .log-msg { flex: 1; }
    .log-info .log-msg { color: #8b949e; }
    .log-warn .log-msg { color: #d29922; }
    .log-error .log-msg { color: #f85149; }
    .log-debug .log-msg { color: #6e7681; }
  `],
})
export class HistoryViewComponent implements OnInit {
  entries = signal<HistoryEntry[]>([]);
  expandedLogs = signal<Record<string, { entries: LogEntry[]; loading: boolean }>>({});

  constructor(private api: ApiService, private snack: MatSnackBar) {}

  ngOnInit(): void { this.load(); }

  load(): void {
    this.api.get<{ entries: HistoryEntry[] }>('/history').subscribe({
      next: r => this.entries.set(r.entries || []),
      error: () => {},
    });
  }

  retry(id: string): void {
    this.api.post(`/history/${id}/retry`).subscribe(() => { this.load(); this.snack.open('Retrying...', 'Close', { duration: 2000 }); });
  }

  remove(id: string): void {
    this.api.delete(`/history/${id}`).subscribe(() => this.load());
  }

  clearAll(): void {
    this.api.delete('/history').subscribe(() => { this.load(); this.snack.open('History cleared', 'Close', { duration: 2000 }); });
  }

  toggleLogs(id: string): void {
    const current = this.expandedLogs();
    if (current[id]) {
      const { [id]: _, ...rest } = current;
      this.expandedLogs.set(rest);
    } else {
      this.expandedLogs.set({ ...current, [id]: { entries: [], loading: true } });
      this.api.get<LogsResponse>(`/history/${id}/logs`).subscribe({
        next: r => {
          const updated = { ...this.expandedLogs() };
          updated[id] = { entries: r.entries || [], loading: false };
          this.expandedLogs.set(updated);
        },
        error: () => {
          const updated = { ...this.expandedLogs() };
          updated[id] = { entries: [], loading: false };
          this.expandedLogs.set(updated);
        },
      });
    }
  }

  formatBytes(b: number): string {
    if (b === 0) return '0 B';
    const k = 1024, s = ['B', 'KB', 'MB', 'GB', 'TB'], i = Math.floor(Math.log(b) / Math.log(k));
    return parseFloat((b / Math.pow(k, i)).toFixed(1)) + ' ' + s[i];
  }
}
