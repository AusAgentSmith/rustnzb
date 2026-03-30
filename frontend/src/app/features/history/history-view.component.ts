import { Component, OnInit, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { MatButtonModule } from '@angular/material/button';
import { MatSnackBar, MatSnackBarModule } from '@angular/material/snack-bar';
import { ApiService } from '../../core/services/api.service';
import { HistoryEntry } from '../../core/models/queue.model';

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
            @if (e.status === 'failed') {
              <button class="btn" (click)="retry(e.id)">🔄 Retry</button>
            }
            <button class="btn" (click)="remove(e.id)">✕</button>
          </div>
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
    .item { display: flex; align-items: center; gap: 12px; padding: 10px 16px; border-bottom: 1px solid #21262d; }
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
  `],
})
export class HistoryViewComponent implements OnInit {
  entries = signal<HistoryEntry[]>([]);

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

  formatBytes(b: number): string {
    if (b === 0) return '0 B';
    const k = 1024, s = ['B', 'KB', 'MB', 'GB', 'TB'], i = Math.floor(Math.log(b) / Math.log(k));
    return parseFloat((b / Math.pow(k, i)).toFixed(1)) + ' ' + s[i];
  }
}
