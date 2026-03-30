import { Component, OnInit, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MatSnackBar, MatSnackBarModule } from '@angular/material/snack-bar';
import { ApiService } from '../../core/services/api.service';

interface RssFeed {
  name: string; url: string; poll_interval_secs: number; category: string | null;
  filter_regex: string | null; enabled: boolean; auto_download: boolean;
}

interface RssItem {
  id: string; feed_name: string; title: string; url: string | null;
  published_at: string | null; downloaded: boolean; category: string; size_bytes: number;
}

interface RssRule {
  id: string; name: string; feed_names: string[]; category: string | null;
  priority: number; match_regex: string; enabled: boolean;
}

interface FeedFormModel {
  name: string; url: string; poll_interval_secs: number; category: string;
  filter_regex: string; enabled: boolean; auto_download: boolean;
}

interface RuleFormModel {
  name: string; match_regex: string; category: string; priority: number;
  enabled: boolean; feed_names_csv: string;
}

@Component({
  selector: 'app-rss-view',
  standalone: true,
  imports: [CommonModule, FormsModule, MatButtonModule, MatSnackBarModule],
  template: `
    <div class="rss-layout">
      <div class="toolbar">
        <span class="title">RSS Feeds</span>
        <span class="spacer"></span>
      </div>

      <!-- Feeds Section -->
      <div class="section">
        <div class="section-header">
          <h3>Feeds</h3>
          @if (!feedFormVisible()) {
            <button class="btn btn-primary" (click)="showAddFeed()">Add Feed</button>
          }
        </div>

        @if (feedFormVisible()) {
          <div class="inline-form">
            <div class="form-title">{{ editingFeedName() ? 'Edit Feed' : 'Add Feed' }}</div>
            <div class="form-row">
              <div class="form-field">
                <label>Name</label>
                <input type="text" [(ngModel)]="feedForm.name" [disabled]="!!editingFeedName()" placeholder="Feed name" />
              </div>
              <div class="form-field form-field-wide">
                <label>URL</label>
                <input type="text" [(ngModel)]="feedForm.url" placeholder="https://..." />
              </div>
            </div>
            <div class="form-row">
              <div class="form-field">
                <label>Poll Interval (seconds)</label>
                <input type="number" [(ngModel)]="feedForm.poll_interval_secs" />
              </div>
              <div class="form-field">
                <label>Category</label>
                <input type="text" [(ngModel)]="feedForm.category" placeholder="Optional" />
              </div>
              <div class="form-field form-field-wide">
                <label>Filter Regex</label>
                <input type="text" [(ngModel)]="feedForm.filter_regex" placeholder="Optional" />
              </div>
            </div>
            <div class="form-row">
              <label class="checkbox-label">
                <input type="checkbox" [(ngModel)]="feedForm.enabled" />
                Enabled
              </label>
              <label class="checkbox-label">
                <input type="checkbox" [(ngModel)]="feedForm.auto_download" />
                Auto Download
              </label>
              <span class="spacer"></span>
              <button class="btn" (click)="cancelFeedForm()">Cancel</button>
              <button class="btn btn-primary" (click)="saveFeed()">{{ editingFeedName() ? 'Update' : 'Add' }}</button>
            </div>
          </div>
        }

        @for (f of feeds(); track f.name) {
          <div class="card">
            <div class="card-info">
              <div class="card-name">{{ f.name }}</div>
              <div class="card-detail">
                {{ f.url }} · Every {{ f.poll_interval_secs }}s · {{ f.category || 'No category' }}
                @if (f.filter_regex) { · Filter: <code>{{ f.filter_regex }}</code> }
                @if (f.auto_download) { · Auto-download }
              </div>
            </div>
            <div class="card-status">
              <span class="dot" [class.active]="f.enabled"></span>
              {{ f.enabled ? 'Active' : 'Disabled' }}
            </div>
            <button class="btn" (click)="editFeed(f)">Edit</button>
            <button class="btn btn-danger" (click)="deleteFeed(f.name)">Delete</button>
          </div>
        }
        @if (feeds().length === 0 && !feedFormVisible()) {
          <div class="empty">No RSS feeds configured.</div>
        }
      </div>

      <!-- Rules Section -->
      <div class="section">
        <div class="section-header">
          <h3>Download Rules</h3>
          @if (!ruleFormVisible()) {
            <button class="btn btn-primary" (click)="showAddRule()">Add Rule</button>
          }
        </div>

        @if (ruleFormVisible()) {
          <div class="inline-form">
            <div class="form-title">{{ editingRuleId() ? 'Edit Rule' : 'Add Rule' }}</div>
            <div class="form-row">
              <div class="form-field">
                <label>Name</label>
                <input type="text" [(ngModel)]="ruleForm.name" placeholder="Rule name" />
              </div>
              <div class="form-field form-field-wide">
                <label>Match Regex</label>
                <input type="text" [(ngModel)]="ruleForm.match_regex" placeholder=".*pattern.*" />
              </div>
            </div>
            <div class="form-row">
              <div class="form-field">
                <label>Category</label>
                <input type="text" [(ngModel)]="ruleForm.category" placeholder="Optional" />
              </div>
              <div class="form-field">
                <label>Priority</label>
                <input type="number" [(ngModel)]="ruleForm.priority" />
              </div>
              <div class="form-field form-field-wide">
                <label>Feed Names (comma-separated)</label>
                <input type="text" [(ngModel)]="ruleForm.feed_names_csv" placeholder="feed1, feed2" />
              </div>
            </div>
            <div class="form-row">
              <label class="checkbox-label">
                <input type="checkbox" [(ngModel)]="ruleForm.enabled" />
                Enabled
              </label>
              <span class="spacer"></span>
              <button class="btn" (click)="cancelRuleForm()">Cancel</button>
              <button class="btn btn-primary" (click)="saveRule()">{{ editingRuleId() ? 'Update' : 'Add' }}</button>
            </div>
          </div>
        }

        @for (r of rules(); track r.id) {
          <div class="card">
            <div class="card-info">
              <div class="card-name">{{ r.name }}</div>
              <div class="card-detail">
                Regex: <code>{{ r.match_regex }}</code> · {{ r.category || 'Any' }} · Priority: {{ r.priority }}
                @if (r.feed_names.length) { · Feeds: {{ r.feed_names.join(', ') }} }
              </div>
            </div>
            <div class="card-status">
              <span class="dot" [class.active]="r.enabled"></span>
              {{ r.enabled ? 'Active' : 'Disabled' }}
            </div>
            <button class="btn" (click)="editRule(r)">Edit</button>
            <button class="btn btn-danger" (click)="deleteRule(r.id)">Delete</button>
          </div>
        }
        @if (rules().length === 0 && !ruleFormVisible()) {
          <div class="empty">No download rules configured.</div>
        }
      </div>

      <!-- Items Section -->
      <div class="section">
        <h3>Recent Items ({{ items().length }})</h3>
        <div class="items-list">
          @for (i of items(); track i.id) {
            <div class="item-row">
              <span class="item-title">{{ i.title }}</span>
              <span class="item-meta">{{ i.feed_name }} · {{ formatBytes(i.size_bytes) }}</span>
              @if (!i.downloaded) {
                <button class="btn" (click)="downloadItem(i.id)">Download</button>
              } @else {
                <span class="downloaded">Done</span>
              }
            </div>
          }
          @if (items().length === 0) {
            <div class="empty">No recent items.</div>
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
    .section-header { display: flex; align-items: center; gap: 12px; margin-bottom: 8px; }
    .section-header h3 { font-size: 14px; margin: 0; color: #c9d1d9; }
    h3 { font-size: 14px; margin-bottom: 8px; color: #c9d1d9; }

    .card { display: flex; align-items: center; gap: 12px; padding: 10px 14px; background: #161b22; border: 1px solid #30363d; border-radius: 6px; margin-bottom: 6px; }
    .card-info { flex: 1; min-width: 0; }
    .card-name { font-weight: 600; font-size: 13px; }
    .card-detail { font-size: 11px; color: #8b949e; margin-top: 2px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .card-detail code { color: #58a6ff; }
    .card-status { display: flex; align-items: center; gap: 4px; font-size: 11px; color: #8b949e; white-space: nowrap; }
    .dot { width: 8px; height: 8px; border-radius: 50%; background: #484f58; }
    .dot.active { background: #3fb950; }

    .btn { padding: 4px 10px; border-radius: 4px; border: 1px solid #30363d; background: #21262d; color: #c9d1d9; cursor: pointer; font-size: 12px; white-space: nowrap; }
    .btn:hover { background: #30363d; }
    .btn-primary { background: #238636; border-color: #2ea043; }
    .btn-primary:hover { background: #2ea043; }
    .btn-danger { color: #f85149; }
    .btn-danger:hover { background: #30363d; }

    .inline-form { padding: 14px; background: #161b22; border: 1px solid #30363d; border-radius: 6px; margin-bottom: 10px; }
    .form-title { font-size: 13px; font-weight: 600; margin-bottom: 10px; color: #c9d1d9; }
    .form-row { display: flex; align-items: flex-end; gap: 10px; margin-bottom: 10px; flex-wrap: wrap; }
    .form-row:last-child { margin-bottom: 0; }
    .form-field { display: flex; flex-direction: column; gap: 4px; min-width: 140px; }
    .form-field-wide { flex: 1; min-width: 200px; }
    .form-field label { font-size: 11px; color: #8b949e; }
    .form-field input[type="text"],
    .form-field input[type="number"] { padding: 6px 10px; background: #0d1117; border: 1px solid #30363d; border-radius: 4px; color: #c9d1d9; font-size: 13px; }
    .form-field input:focus { border-color: #58a6ff; outline: none; }
    .form-field input:disabled { opacity: 0.5; cursor: not-allowed; }

    .checkbox-label { display: flex; align-items: center; gap: 6px; font-size: 12px; color: #c9d1d9; cursor: pointer; padding-bottom: 4px; }
    .checkbox-label input[type="checkbox"] { accent-color: #58a6ff; }

    .empty { padding: 16px; color: #484f58; font-size: 12px; }
    .items-list { max-height: 300px; overflow-y: auto; }
    .item-row { display: flex; align-items: center; gap: 8px; padding: 5px 0; border-bottom: 1px solid #21262d; font-size: 12px; }
    .item-title { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .item-meta { color: #8b949e; font-size: 11px; white-space: nowrap; }
    .downloaded { color: #3fb950; font-size: 12px; }
  `],
})
export class RssViewComponent implements OnInit {
  feeds = signal<RssFeed[]>([]);
  rules = signal<RssRule[]>([]);
  items = signal<RssItem[]>([]);

  // Feed form state
  feedFormVisible = signal(false);
  editingFeedName = signal<string | null>(null);
  feedForm: FeedFormModel = this.emptyFeedForm();

  // Rule form state
  ruleFormVisible = signal(false);
  editingRuleId = signal<string | null>(null);
  ruleForm: RuleFormModel = this.emptyRuleForm();

  constructor(private api: ApiService, private snack: MatSnackBar) {}

  ngOnInit(): void { this.loadAll(); }

  loadAll(): void {
    this.api.get<RssFeed[]>('/config/rss-feeds').subscribe({
      next: feeds => this.feeds.set(feeds),
      error: () => {},
    });
    this.api.get<RssRule[]>('/rss/rules').subscribe({
      next: rules => this.rules.set(rules),
      error: () => {},
    });
    this.api.get<RssItem[]>('/rss/items').subscribe({
      next: items => this.items.set(items),
      error: () => {},
    });
  }

  // -- Feed CRUD --

  showAddFeed(): void {
    this.feedForm = this.emptyFeedForm();
    this.editingFeedName.set(null);
    this.feedFormVisible.set(true);
  }

  editFeed(f: RssFeed): void {
    this.feedForm = {
      name: f.name,
      url: f.url,
      poll_interval_secs: f.poll_interval_secs,
      category: f.category || '',
      filter_regex: f.filter_regex || '',
      enabled: f.enabled,
      auto_download: f.auto_download,
    };
    this.editingFeedName.set(f.name);
    this.feedFormVisible.set(true);
  }

  cancelFeedForm(): void {
    this.feedFormVisible.set(false);
    this.editingFeedName.set(null);
  }

  saveFeed(): void {
    const body: RssFeed = {
      name: this.feedForm.name.trim(),
      url: this.feedForm.url.trim(),
      poll_interval_secs: this.feedForm.poll_interval_secs,
      category: this.feedForm.category.trim() || null,
      filter_regex: this.feedForm.filter_regex.trim() || null,
      enabled: this.feedForm.enabled,
      auto_download: this.feedForm.auto_download,
    };
    if (!body.name || !body.url) {
      this.snack.open('Name and URL are required', 'Close', { duration: 3000 });
      return;
    }
    const editing = this.editingFeedName();
    const req = editing
      ? this.api.put(`/config/rss-feeds/${encodeURIComponent(editing)}`, body)
      : this.api.post('/config/rss-feeds', body);
    req.subscribe({
      next: () => {
        this.snack.open(editing ? 'Feed updated' : 'Feed added', 'Close', { duration: 2000 });
        this.cancelFeedForm();
        this.loadAll();
      },
      error: () => this.snack.open('Failed to save feed', 'Close', { duration: 3000 }),
    });
  }

  deleteFeed(name: string): void {
    this.api.delete(`/config/rss-feeds/${encodeURIComponent(name)}`).subscribe({
      next: () => {
        this.snack.open('Feed deleted', 'Close', { duration: 2000 });
        this.loadAll();
      },
      error: () => this.snack.open('Failed to delete feed', 'Close', { duration: 3000 }),
    });
  }

  // -- Rule CRUD --

  showAddRule(): void {
    this.ruleForm = this.emptyRuleForm();
    this.editingRuleId.set(null);
    this.ruleFormVisible.set(true);
  }

  editRule(r: RssRule): void {
    this.ruleForm = {
      name: r.name,
      match_regex: r.match_regex,
      category: r.category || '',
      priority: r.priority,
      enabled: r.enabled,
      feed_names_csv: r.feed_names.join(', '),
    };
    this.editingRuleId.set(r.id);
    this.ruleFormVisible.set(true);
  }

  cancelRuleForm(): void {
    this.ruleFormVisible.set(false);
    this.editingRuleId.set(null);
  }

  saveRule(): void {
    const feedNames = this.ruleForm.feed_names_csv
      .split(',')
      .map(s => s.trim())
      .filter(s => s.length > 0);
    const body = {
      name: this.ruleForm.name.trim(),
      match_regex: this.ruleForm.match_regex.trim(),
      category: this.ruleForm.category.trim() || null,
      priority: this.ruleForm.priority,
      enabled: this.ruleForm.enabled,
      feed_names: feedNames,
    };
    if (!body.name || !body.match_regex) {
      this.snack.open('Name and match regex are required', 'Close', { duration: 3000 });
      return;
    }
    const editing = this.editingRuleId();
    const req = editing
      ? this.api.put(`/rss/rules/${editing}`, body)
      : this.api.post('/rss/rules', body);
    req.subscribe({
      next: () => {
        this.snack.open(editing ? 'Rule updated' : 'Rule added', 'Close', { duration: 2000 });
        this.cancelRuleForm();
        this.loadAll();
      },
      error: () => this.snack.open('Failed to save rule', 'Close', { duration: 3000 }),
    });
  }

  deleteRule(id: string): void {
    this.api.delete(`/rss/rules/${id}`).subscribe({
      next: () => {
        this.snack.open('Rule deleted', 'Close', { duration: 2000 });
        this.loadAll();
      },
      error: () => this.snack.open('Failed to delete rule', 'Close', { duration: 3000 }),
    });
  }

  // -- Items --

  downloadItem(id: string): void {
    this.api.post(`/rss/items/${id}/download`).subscribe({
      next: () => {
        this.snack.open('Added to queue', 'Close', { duration: 2000 });
        this.loadAll();
      },
      error: () => this.snack.open('Download failed', 'Close', { duration: 3000 }),
    });
  }

  // -- Helpers --

  formatBytes(b: number): string {
    if (!b) return '';
    const k = 1024;
    const s = ['B', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(b) / Math.log(k));
    return parseFloat((b / Math.pow(k, i)).toFixed(1)) + ' ' + s[i];
  }

  private emptyFeedForm(): FeedFormModel {
    return { name: '', url: '', poll_interval_secs: 900, category: '', filter_regex: '', enabled: true, auto_download: false };
  }

  private emptyRuleForm(): RuleFormModel {
    return { name: '', match_regex: '', category: '', priority: 1, enabled: true, feed_names_csv: '' };
  }
}
