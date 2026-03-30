import { Injectable } from '@angular/core';
import { HttpClient } from '@angular/common/http';
import { Observable, tap } from 'rxjs';

export interface AuthStatus {
  auth_enabled: boolean;
  setup_required: boolean;
}

export interface TokenResponse {
  access_token: string;
  refresh_token: string;
  token_type: string;
  expires_in: number;
}

@Injectable({ providedIn: 'root' })
export class AuthService {
  private baseUrl = '/api/auth';

  constructor(private http: HttpClient) {}

  checkAuth(): Observable<AuthStatus> {
    return this.http.get<AuthStatus>(`${this.baseUrl}/status`);
  }

  setup(username: string, password: string): Observable<TokenResponse> {
    return this.http
      .post<TokenResponse>(`${this.baseUrl}/setup`, { username, password })
      .pipe(tap((res) => this.storeTokens(res)));
  }

  login(username: string, password: string): Observable<TokenResponse> {
    return this.http
      .post<TokenResponse>(`${this.baseUrl}/login`, { username, password })
      .pipe(tap((res) => this.storeTokens(res)));
  }

  refresh(): Observable<TokenResponse> {
    const refreshToken = localStorage.getItem('refresh_token');
    return this.http
      .post<TokenResponse>(`${this.baseUrl}/refresh`, { refresh_token: refreshToken })
      .pipe(tap((res) => this.storeTokens(res)));
  }

  logout(): Observable<void> {
    const refreshToken = localStorage.getItem('refresh_token');
    this.clearTokens();
    return this.http.post<void>(`${this.baseUrl}/logout`, { refresh_token: refreshToken });
  }

  isLoggedIn(): boolean {
    return !!localStorage.getItem('access_token');
  }

  getAccessToken(): string | null {
    return localStorage.getItem('access_token');
  }

  private storeTokens(res: TokenResponse): void {
    localStorage.setItem('access_token', res.access_token);
    localStorage.setItem('refresh_token', res.refresh_token);
  }

  clearTokens(): void {
    localStorage.removeItem('access_token');
    localStorage.removeItem('refresh_token');
  }
}
