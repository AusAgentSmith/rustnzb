import { Component, OnInit, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { MatButtonModule } from '@angular/material/button';
import { MatSnackBar, MatSnackBarModule } from '@angular/material/snack-bar';
import { ApiService } from '../../core/services/api.service';

interface RssFeed { name: string; url: string; poll_interval_secs: number; category: string; enabled: boolean; }
interface RssItem { id: string; feed_name: string; title: string; url: string | null; published_at: string | null; downloaded: boolean; category: string; size_bytes: number; }
interface RssRule { id: string; name: string; feed_names: string[]; category: string | null; priority: number; match_regex: string; enabled: boolean; }

@Component({
  selector: 'app-rss-view',
  standalone: true,
  imports: [CommonModule, MatButtonModule, MatSnackBarModule],
  template: `
    <div class="rss-layout">
      <div class="toolbar">
        <span class="title">RSS Feeds</span>
        <span class="spacer"></span>
      </div>

      <div class="section">
        <h3>Feeds</h3>
        @for (f of feeds(); track f.name) {
          <div class="card">
            <div class="card-info">
              <div class="card-name">{{ f.name }}</div>
              <div class="card-detail">{{ f.url }} · Every {{ f.poll_interval_secs }}s · {{ f.category || 'No category' }}</div>
            </div>
            <div class="card-status">
              <span class="dot" [class.active]="f.enabled"></span>
              {{ f.enabled ? 'Active' : 'Disabled' }}
            </div>
          </div>
        }
        @if (feeds().length === 0) { <div class="empty">No RSS feeds configured. Add them in Settings.</div> }
      </div>

      <div class="section">
        <h3>Download Rules</h3>
        @for (r of rules(); track r.id) {
          <div class="card">
            <div class="card-info">
              <div class="card-name">{{ r.name }}</div>
              <div class="card-detail">Regex: <code>{{ r.match_regex }}</code> · {{ r.category || 'Any' }} · Priority: {{ r.priority }}</div>
            </div>
            <div class="card-status">
              <span class="dot" [class.active]="r.enabled"></span>
              {{ r.enabled ? 'Active' : 'Disabled' }}
            </div>
          </div>
        }
        @if (rules().length === 0) { <div class="empty">No download rules configured.</div> }
      </div>

      <div class="section">
        <h3>Recent Items ({{ items().length }})</h3>
        <div class="items-list">
          @for (i of items(); track i.id) {
            <div class="item-row">
              <span class="item-title">{{ i.title }}</span>
              <span class="item-meta">{{ i.feed_name }} · {{ formatBytes(i.size_bytes) }}</span>
              @if (!i.downloaded) {
                <button class="btn" (click)="downloadItem(i.id)">↓</button>
              } @else {
                <span class="downloaded">✓</span>
              }
            </div>
          }
        </div>
      </div>
    </div>
  `,
  styles: [`
    :host { display: flex; height: 100%; overflow-y: auto; }
    .rss-layout { flex: 1; padding: 16px 24px; }
    .toolbar { display: flex; align-items: center; margin-bottom: 16px; }
    .title { font-size: 16px; font-weight: 600; }
    .spacer { flex: 1; }
    .section { margin-bottom: 24px; }
    h3 { font-size: 14px; margin-bottom: 8px; color: #c9d1d9; }
    .card { display: flex; align-items: center; gap: 12px; padding: 10px 14px; background: #161b22; border: 1px solid #30363d; border-radius: 6px; margin-bottom: 6px; }
    .card-info { flex: 1; }
    .card-name { font-weight: 600; font-size: 13px; }
    .card-detail { font-size: 11px; color: #8b949e; margin-top: 2px; }
    .card-detail code { color: #58a6ff; }
    .card-status { display: flex; align-items: center; gap: 4px; font-size: 11px; color: #8b949e; }
    .dot { width: 8px; height: 8px; border-radius: 50%; background: #484f58; }
    .dot.active { background: #3fb950; }
    .btn { padding: 3px 8px; border-radius: 4px; border: 1px solid #30363d; background: #21262d; color: #c9d1d9; cursor: pointer; font-size: 12px; }
    .btn:hover { background: #30363d; }
    .empty { padding: 16px; color: #484f58; font-size: 12px; }
    .items-list { max-height: 300px; overflow-y: auto; }
    .item-row { display: flex; align-items: center; gap: 8px; padding: 5px 0; border-bottom: 1px solid #21262d; font-size: 12px; }
    .item-title { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .item-meta { color: #8b949e; font-size: 11px; white-space: nowrap; }
    .downloaded { color: #3fb950; font-size: 14px; }
  `],
})
export class RssViewComponent implements OnInit {
  feeds = signal<RssFeed[]>([]);
  rules = signal<RssRule[]>([]);
  items = signal<RssItem[]>([]);

  constructor(private api: ApiService, private snack: MatSnackBar) {}

  ngOnInit(): void { this.loadAll(); }

  loadAll(): void {
    this.api.get<{ feeds: RssFeed[] }>('/config/rss-feeds').subscribe({ next: r => this.feeds.set(r.feeds || []), error: () => {} });
    this.api.get<{ rules: RssRule[] }>('/rss/rules').subscribe({ next: r => this.rules.set(r.rules || []), error: () => {} });
    this.api.get<{ items: RssItem[] }>('/rss/items').subscribe({ next: r => this.items.set(r.items || []), error: () => {} });
  }

  downloadItem(id: string): void {
    this.api.post(`/rss/items/${id}/download`).subscribe({
      next: () => { this.snack.open('Added to queue', 'Close', { duration: 2000 }); this.loadAll(); },
      error: () => this.snack.open('Download failed', 'Close', { duration: 3000 }),
    });
  }

  formatBytes(b: number): string {
    if (!b) return '';
    const k = 1024, s = ['B', 'KB', 'MB', 'GB'], i = Math.floor(Math.log(b) / Math.log(k));
    return parseFloat((b / Math.pow(k, i)).toFixed(1)) + ' ' + s[i];
  }
}
