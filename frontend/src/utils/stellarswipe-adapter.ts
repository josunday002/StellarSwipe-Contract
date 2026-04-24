export interface StellarSwipeStats {
  cash: number;
  incomeRate: number;
  boosts: number;
}

export type FetchErrorKind = 'network' | 'server';

export class FetchError extends Error {
  constructor(public kind: FetchErrorKind, message: string) {
    super(message);
    this.name = 'FetchError';
  }
}

export class StellarSwipeHUDAdapter {
  private contractAddress: string;
  private networkUrl: string;

  constructor(contractAddress: string, networkUrl: string = 'https://soroban-testnet.stellar.org') {
    this.contractAddress = contractAddress;
    this.networkUrl = networkUrl;
  }

  async fetchTycoonStats(): Promise<StellarSwipeStats> {
    let response: Response;
    try {
      response = await fetch(`${this.networkUrl}/contracts/${this.contractAddress}/stats`);
    } catch {
      throw new FetchError('network', 'Unable to reach the server. Check your connection.');
    }
    if (!response.ok) {
      throw new FetchError('server', `Server error ${response.status}: ${response.statusText}`);
    }
    const data = await response.json();
    return {
      cash: data.cash || 0,
      incomeRate: data.income_rate || 0,
      boosts: data.active_boosts || 0,
    };
  }

  async batchFetchStats(requests: string[]): Promise<StellarSwipeStats[]> {
    let batchResponse: Response;
    try {
      batchResponse = await fetch(`${this.networkUrl}/batch`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ requests }),
      });
    } catch {
      throw new FetchError('network', 'Unable to reach the server. Check your connection.');
    }
    if (!batchResponse.ok) {
      throw new FetchError('server', `Batch request failed: ${batchResponse.status}`);
    }
    return batchResponse.json();
  }
}
