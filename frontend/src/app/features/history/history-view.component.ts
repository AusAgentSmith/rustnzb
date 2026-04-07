import { Component, OnInit, OnDestroy, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { MatIconModule } from '@angular/material/icon';
import { MatSnackBar, MatSnackBarModule } from '@angular/material/snack-bar';
import { ApiService } from '../../core/services/api.service';
import { HistoryEntry, LogEntry, LogsResponse, StageResult, ServerArticleStats } from '../../core/models/queue.model';

@Component({
  selector: 'app-history-view',
  standalone: true,
  imports: [CommonModule, MatIconModule, MatSnackBarModule],
  template: `
    <div class="split-layout">
      <!-- Left: History list -->
      <div class="list-panel">
        <div class="filter-bar">
          <button class="filter-chip" [class.active]="filterStatus === 'all'" (click)="filterStatus = 'all'">All ({{ entries().length }})</button>
          <button class="filter-chip" [class.active]="filterStatus === 'completed'" (click)="filterStatus = 'completed'">Completed</button>
          <button class="filter-chip" [class.active]="filterStatus === 'failed'" (click)="filterStatus = 'failed'">Failed</button>
          <span class="filter-spacer"></span>
          @if (entries().length > 0) {
            <button class="btn btn-danger-sm" (click)="clearAll()">Clear All</button>
          }
        </div>

        <div class="job-list">
          @for (e of filteredEntries(); track e.id) {
            <div class="job-row" [class.selected]="selectedId() === e.id" (click)="select(e.id)">
              <div class="job-status-icon" [class]="e.status">
                <mat-icon>{{ e.status === 'completed' ? 'check' : 'error_outline' }}</mat-icon>
              </div>
              <div class="job-info">
                <div class="job-name">{{ e.name }}</div>
                <div class="job-subtitle">
                  <span>{{ formatBytes(e.total_bytes) }}</span>
                  @if (e.category) { <span>{{ e.category }}</span> }
                  <span>{{ formatDate(e.completed_at) }}</span>
                </div>
              </div>
              <div class="job-right">
                <div class="job-status-text" [class]="e.status">{{ e.status === 'completed' ? 'Done' : 'Failed' }}</div>
              </div>
            </div>
          }

          @if (entries().length === 0) {
            <div class="empty-state">
              <mat-icon class="empty-icon">history</mat-icon>
              <div class="empty-text">No download history</div>
            </div>
          }
        </div>
      </div>

      <!-- Right: Detail panel -->
      <div class="detail-panel">
        @if (selectedEntry(); as e) {
          <div class="detail-header">
            <div class="detail-title">{{ e.name }}</div>
            <div class="detail-status-row">
              <span class="tag" [class]="'tag-' + e.status">{{ e.status | uppercase }}</span>
              @if (e.category) { <span class="tag tag-cat">{{ e.category }}</span> }
            </div>
          </div>

          <div class="detail-tabs">
            <button class="dtab" [class.active]="detailTab() === 'info'" (click)="detailTab.set('info')">Info</button>
            <button class="dtab" [class.active]="detailTab() === 'logs'" (click)="detailTab.set('logs'); loadLogs(e.id)">Logs</button>
          </div>

          <div class="detail-body">
            @if (detailTab() === 'info') {
              <div class="detail-row"><span class="dr-label">Status</span><span class="dr-value">{{ e.status | uppercase }}</span></div>
              <div class="detail-row"><span class="dr-label">Size</span><span class="dr-value">{{ formatBytes(e.total_bytes) }}</span></div>
              <div class="detail-row"><span class="dr-label">Downloaded</span><span class="dr-value">{{ formatBytes(e.downloaded_bytes) }}</span></div>
              <div class="detail-row"><span class="dr-label">Added</span><span class="dr-value">{{ formatDate(e.added_at) }}</span></div>
              <div class="detail-row"><span class="dr-label">Completed</span><span class="dr-value">{{ formatDate(e.completed_at) }}</span></div>
              <div class="detail-row"><span class="dr-label">Duration</span><span class="dr-value">{{ formatDuration(e.added_at, e.completed_at) }}</span></div>
              <div class="detail-row"><span class="dr-label">Avg Speed</span><span class="dr-value">{{ avgSpeed(e) }}</span></div>
              <div class="detail-row"><span class="dr-label">Category</span><span class="dr-value">{{ e.category || 'None' }}</span></div>
              <div class="detail-row"><span class="dr-label">Output</span><span class="dr-value">{{ e.output_dir }}</span></div>
              @if (e.error_message) {
                <div class="detail-row"><span class="dr-label">Error</span><span class="dr-value error-text">{{ e.error_message }}</span></div>
              }

              @if (e.stages && e.stages.length > 0) {
                <div class="detail-section-title">Processing Stages</div>
                @for (s of e.stages; track s.name) {
                  <div class="stage-row">
                    <span class="stage-name">{{ s.name }}</span>
                    <span class="tag" [class]="'tag-' + s.status">{{ s.status }}</span>
                    <span class="stage-dur">{{ s.duration_secs.toFixed(1) }}s</span>
                    @if (s.message) { <span class="stage-msg">{{ s.message }}</span> }
                  </div>
                }
              }

              @if (e.server_stats && e.server_stats.length > 0) {
                <div class="detail-section-title">Server Stats</div>
                @for (ss of e.server_stats; track ss.server_id) {
                  <div class="server-stat">
                    <span class="server-dot" [style.background]="serverColor(ss.server_id)"></span>
                    <span class="server-name">{{ ss.server_name }}</span>
                    <span class="server-count">{{ ss.articles_downloaded }} articles &middot; {{ formatBytes(ss.bytes_downloaded) }}</span>
                  </div>
                }
              }
            }
            @if (detailTab() === 'logs') {
              <div class="log-viewer">
                @if (logData().loading) { <div class="log-empty">Loading logs...</div> }
                @else if (logData().entries.length === 0) { <div class="log-empty">No log entries</div> }
                @else {
                  @for (log of logData().entries; track log.seq) {
                    <div class="log-line" [class]="'log-' + log.level.toLowerCase()">
                      <span class="log-ts">{{ log.timestamp | slice:11:19 }}</span>
                      <span class="log-msg">{{ log.message }}</span>
                    </div>
                  }
                }
              </div>
            }
          </div>

          <div class="detail-actions">
            @if (e.status === 'failed') {
              <button class="btn" (click)="retry(e.id)"><mat-icon>replay</mat-icon> Retry</button>
            }
            <span class="spacer"></span>
            <button class="btn btn-danger" (click)="remove(e.id)"><mat-icon>delete_outline</mat-icon></button>
          </div>
        }

        @if (!selectedEntry()) {
          <div class="detail-empty">
            <mat-icon class="detail-empty-icon">info_outline</mat-icon>
            <div>Select an entry to view details</div>
          </div>
        }
      </div>
    </div>
  `,
  styles: [`
    :host { display: flex; flex-direction: column; height: 100%; overflow: hidden; }

    .split-layout { flex: 1; display: flex; overflow: hidden; }
    .list-panel { flex: 1; display: flex; flex-direction: column; border-right: 1px solid #21262d; min-width: 0; }

    .filter-bar {
      display: flex; align-items: center; gap: 6px; padding: 8px 14px;
      border-bottom: 1px solid #21262d; background: #0d1117; flex-shrink: 0;
    }
    .filter-chip {
      padding: 4px 12px; border-radius: 14px; border: 1px solid #30363d;
      background: transparent; color: #8b949e; cursor: pointer; font-size: 12px; transition: all 0.15s;
    }
    .filter-chip:hover { border-color: #484f58; color: #c9d1d9; }
    .filter-chip.active { border-color: #f0883e; color: #f0883e; background: #f0883e11; }
    .filter-spacer { flex: 1; }
    .btn-danger-sm {
      padding: 3px 10px; border-radius: 4px; border: 1px solid #da3634;
      background: transparent; color: #f85149; cursor: pointer; font-size: 11px;
    }
    .btn-danger-sm:hover { background: #da363422; }

    .job-list { flex: 1; overflow-y: auto; }
    .job-row {
      display: flex; align-items: center; gap: 12px; padding: 10px 14px;
      border-bottom: 1px solid #161b22; cursor: pointer; transition: all 0.1s;
    }
    .job-row:hover { background: #161b22; }
    .job-row.selected { background: #161b22; border-left: 3px solid #f0883e; padding-left: 11px; }

    .job-status-icon {
      width: 32px; height: 32px; border-radius: 6px;
      display: flex; align-items: center; justify-content: center; flex-shrink: 0;
    }
    .job-status-icon mat-icon { font-size: 18px; width: 18px; height: 18px; }
    .job-status-icon.completed { background: #23863633; color: #3fb950; }
    .job-status-icon.failed { background: #da363433; color: #f85149; }

    .job-info { flex: 1; min-width: 0; }
    .job-name { font-size: 13px; font-weight: 600; color: #8b949e; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .job-subtitle { font-size: 11px; color: #484f58; margin-top: 2px; display: flex; align-items: center; gap: 6px; }

    .job-right { flex-shrink: 0; }
    .job-status-text { font-size: 12px; font-weight: 500; }
    .job-status-text.completed { color: #3fb950; }
    .job-status-text.failed { color: #f85149; }

    .empty-state { text-align: center; padding: 48px 16px; color: #484f58; }
    .empty-icon { font-size: 48px !important; width: 48px !important; height: 48px !important; color: #21262d; margin-bottom: 12px; }
    .empty-text { font-size: 14px; font-weight: 600; }

    .detail-panel {
      width: 380px; overflow-y: auto; background: #010409; flex-shrink: 0;
      display: flex; flex-direction: column;
    }
    .detail-header { padding: 18px 20px 14px; border-bottom: 1px solid #21262d; }
    .detail-title { font-size: 15px; font-weight: 700; color: #e6edf3; margin-bottom: 8px; word-break: break-word; }
    .detail-status-row { display: flex; align-items: center; gap: 8px; }

    .detail-tabs { display: flex; border-bottom: 1px solid #21262d; padding: 0 20px; }
    .dtab {
      padding: 8px 14px; border-bottom: 2px solid transparent;
      background: none; border-top: none; border-left: none; border-right: none;
      color: #8b949e; cursor: pointer; font-size: 12px; font-weight: 500;
    }
    .dtab:hover { color: #c9d1d9; }
    .dtab.active { color: #e6edf3; border-bottom-color: #f0883e; }

    .detail-body { flex: 1; padding: 14px 20px; overflow-y: auto; }
    .detail-row { display: flex; justify-content: space-between; padding: 6px 0; border-bottom: 1px solid #161b2244; }
    .dr-label { font-size: 12px; color: #484f58; }
    .dr-value { font-size: 12px; color: #c9d1d9; font-family: 'JetBrains Mono', Consolas, monospace; text-align: right; max-width: 200px; word-break: break-all; }
    .error-text { color: #f85149; }

    .tag { font-size: 10px; font-weight: 600; padding: 2px 8px; border-radius: 10px; text-transform: uppercase; letter-spacing: 0.3px; }
    .tag-completed, .tag-ok { background: #23863633; color: #3fb950; }
    .tag-failed, .tag-error { background: #da363433; color: #f85149; }
    .tag-skipped { background: #30363d; color: #8b949e; }
    .tag-cat { background: #21262d; color: #8b949e; }

    .detail-section-title {
      margin-top: 16px; margin-bottom: 8px; font-size: 11px; font-weight: 600;
      color: #484f58; text-transform: uppercase; letter-spacing: 0.5px;
    }
    .stage-row { display: flex; gap: 10px; align-items: center; padding: 4px 0; font-size: 12px; color: #c9d1d9; }
    .stage-name { min-width: 80px; color: #c9d1d9; }
    .stage-dur { color: #8b949e; font-family: 'JetBrains Mono', Consolas, monospace; font-size: 11px; }
    .stage-msg { color: #484f58; font-size: 11px; }

    .server-stat { display: flex; align-items: center; gap: 8px; padding: 5px 0; font-size: 12px; }
    .server-dot { width: 8px; height: 8px; border-radius: 50%; flex-shrink: 0; }
    .server-name { color: #c9d1d9; flex: 1; }
    .server-count { color: #8b949e; font-family: 'JetBrains Mono', Consolas, monospace; font-size: 11px; }

    .log-viewer {
      font-family: 'JetBrains Mono', Consolas, monospace; font-size: 11px;
      background: #0d1117; border: 1px solid #21262d; border-radius: 4px; padding: 8px;
      max-height: 400px; overflow-y: auto;
    }
    .log-empty { color: #484f58; padding: 8px 0; font-size: 12px; }
    .log-line { display: flex; gap: 8px; line-height: 1.6; white-space: pre-wrap; word-break: break-all; }
    .log-ts { color: #30363d; flex-shrink: 0; }
    .log-msg { flex: 1; }
    .log-info .log-msg { color: #8b949e; }
    .log-warn .log-msg { color: #d29922; }
    .log-error .log-msg { color: #f85149; }
    .log-debug .log-msg { color: #6e7681; }

    .detail-actions { display: flex; gap: 8px; padding: 14px 20px; border-top: 1px solid #21262d; flex-shrink: 0; }
    .spacer { flex: 1; }

    .detail-empty {
      flex: 1; display: flex; flex-direction: column; align-items: center;
      justify-content: center; color: #30363d; gap: 8px; font-size: 13px;
    }
    .detail-empty-icon { font-size: 40px !important; width: 40px !important; height: 40px !important; }

    .btn {
      padding: 7px 14px; border-radius: 6px; border: 1px solid #30363d;
      background: #21262d; color: #c9d1d9; cursor: pointer; font-size: 12px;
      font-weight: 500; display: flex; align-items: center; gap: 6px;
    }
    .btn:hover { background: #30363d; }
    .btn mat-icon { font-size: 16px; width: 16px; height: 16px; }
    .btn-danger { background: transparent; border-color: #da3634; color: #f85149; }
    .btn-danger:hover { background: #da363422; }
  `],
})
export class HistoryViewComponent implements OnInit, OnDestroy {
  entries = signal<HistoryEntry[]>([]);
  selectedId = signal<string | null>(null);
  detailTab = signal<'info' | 'logs'>('info');
  logData = signal<{ entries: LogEntry[]; loading: boolean }>({ entries: [], loading: false });
  filterStatus: 'all' | 'completed' | 'failed' = 'all';
  private pollTimer: ReturnType<typeof setInterval> | null = null;

  constructor(private api: ApiService, private snack: MatSnackBar) {}

  ngOnInit(): void {
    this.load();
    this.pollTimer = setInterval(() => this.load(), 5000);
  }

  ngOnDestroy(): void {
    if (this.pollTimer) clearInterval(this.pollTimer);
  }

  load(): void {
    this.api.get<{ entries: HistoryEntry[] }>('/history').subscribe({
      next: r => this.entries.set(r.entries || []),
      error: () => {},
    });
  }

  filteredEntries(): HistoryEntry[] {
    const all = this.entries();
    if (this.filterStatus === 'all') return all;
    return all.filter(e => e.status === this.filterStatus);
  }

  selectedEntry(): HistoryEntry | null {
    const id = this.selectedId();
    if (!id) return null;
    return this.entries().find(e => e.id === id) ?? null;
  }

  select(id: string): void {
    if (this.selectedId() === id) { this.selectedId.set(null); return; }
    this.selectedId.set(id);
    this.detailTab.set('info');
    this.logData.set({ entries: [], loading: false });
  }

  loadLogs(id: string): void {
    this.logData.set({ entries: [], loading: true });
    this.api.get<LogsResponse>(`/history/${id}/logs`).subscribe({
      next: r => this.logData.set({ entries: r.entries || [], loading: false }),
      error: () => this.logData.set({ entries: [], loading: false }),
    });
  }

  retry(id: string): void {
    this.api.post(`/history/${id}/retry`).subscribe(() => {
      this.load();
      this.snack.open('Retrying...', 'Close', { duration: 2000 });
    });
  }

  remove(id: string): void {
    this.api.delete(`/history/${id}`).subscribe(() => {
      if (this.selectedId() === id) this.selectedId.set(null);
      this.load();
    });
  }

  clearAll(): void {
    this.api.delete('/history').subscribe(() => {
      this.selectedId.set(null);
      this.load();
      this.snack.open('History cleared', 'Close', { duration: 2000 });
    });
  }

  serverColor(serverId: string): string {
    const colors = ['#3fb950', '#58a6ff', '#f0883e', '#d29922', '#bc8cff', '#f778ba'];
    let hash = 0;
    for (const c of serverId) hash = ((hash << 5) - hash + c.charCodeAt(0)) | 0;
    return colors[Math.abs(hash) % colors.length];
  }

  formatBytes(b: number): string {
    if (b === 0) return '0 B';
    const k = 1024, s = ['B', 'KB', 'MB', 'GB', 'TB'], i = Math.floor(Math.log(b) / Math.log(k));
    return parseFloat((b / Math.pow(k, i)).toFixed(1)) + ' ' + s[i];
  }

  formatDate(d: string): string {
    if (!d) return '--';
    return new Date(d).toLocaleString();
  }

  formatDuration(start: string, end: string): string {
    const ms = new Date(end).getTime() - new Date(start).getTime();
    if (ms <= 0) return '--';
    const secs = Math.floor(ms / 1000);
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    const s = secs % 60;
    if (h > 0) return `${h}h ${m}m ${s}s`;
    if (m > 0) return `${m}m ${s}s`;
    return `${s}s`;
  }

  avgSpeed(e: HistoryEntry): string {
    const start = new Date(e.added_at).getTime();
    const end = new Date(e.completed_at).getTime();
    const durationSecs = (end - start) / 1000;
    if (durationSecs <= 0) return '--';
    const bps = e.downloaded_bytes / durationSecs;
    return this.formatBytes(bps) + '/s';
  }
}
