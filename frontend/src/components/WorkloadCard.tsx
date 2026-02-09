import React, { useState } from 'react';
import { Workload } from '../types';
import { api } from '../api';

interface WorkloadCardProps {
  workload: Workload;
  onUpdate?: () => void;
}

export const WorkloadCard: React.FC<WorkloadCardProps> = ({ workload, onUpdate }) => {
  const [isLoading, setIsLoading] = useState(false);

  const handleRefresh = async () => {
    setIsLoading(true);
    try {
      await api.updateWorkload(workload);
      if (onUpdate) onUpdate();
    } catch (error) {
      console.error('Error updating workload:', error);
    } finally {
      setIsLoading(false);
    }
  };

  const handleUpgrade = async () => {
    setIsLoading(true);
    try {
      await api.upgradeWorkload(workload);
      if (onUpdate) onUpdate();
    } catch (error) {
      console.error('Error upgrading workload:', error);
    } finally {
      setIsLoading(false);
    }
  };

return (
     <div className={`${workload.update_available === 'Available' ? 'workload-card-update-available' : 'workload-card'} ${workload.scan_exhausted === 'true' ? 'workload-card-scan-exhausted' : ''}`}>
      <div className="workload-name">{workload.name}</div>
      <button
        onClick={handleRefresh}
        className="workload-update-single"
        disabled={isLoading}
      >
        Refresh
      </button>
      <div className="workload-namespace">Namespace: {workload.namespace}</div>
      <div className="workload-version">Current Tag {workload.current_version}</div>
      <div className="workload-image">Image: {workload.image}</div>
      <div className="workload-last-scanned">Last Scanned: {workload.last_scanned}</div>
       {workload.scan_exhausted === 'true' && (
         <div className="scan-exhausted">Scan Exhausted</div>
       )}

      {workload.update_available === 'Available' && (
        <>
          <div className="workload-latest-version">Latest Version Available: {workload.latest_version}</div>
          <br />
          <button
            onClick={handleUpgrade}
            className="upgrade-button"
            disabled={isLoading}
          >
            Upgrade
          </button>
        </>
      )}
    </div>
  );
};
